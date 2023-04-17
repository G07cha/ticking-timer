use crossbeam_channel::{bounded, Receiver, SendError, Sender};
use std::{
  sync::{Arc, Condvar, Mutex},
  thread,
  time::{Duration, Instant},
};

pub struct Timer {
  /** Restart/reset timer, if duration is not specified will continue using remaining duration */
  reset_sender: Sender<Option<Duration>>,
  /** Contains isRunning boolean mutex with Condvar for notifying when value changes */
  running_state: Arc<(Mutex<bool>, Condvar)>,
  pub update_receiver: Receiver<Duration>,
}

impl Timer {
  pub fn new(update_frequency: Duration) -> Self {
    let running_state = Arc::new((Mutex::new(false), Condvar::new()));
    let (reset_sender, reset_receiver) = bounded::<Option<Duration>>(0);
    let (update_sender, update_receiver) = bounded::<Duration>(0);
    let mut remaining = Duration::ZERO;

    thread::spawn({
      let running_state = running_state.clone();
      move || {
        while let Ok(reset_payload) = reset_receiver.recv() {
          remaining = reset_payload.unwrap_or(remaining);
          let (lock, cvar) = &*running_state;

          while !remaining.is_zero() {
            let now = Instant::now();

            let (is_running, _) = cvar
              .wait_timeout(lock.lock().unwrap(), update_frequency)
              .unwrap();

            if !*is_running {
              break;
            }

            remaining = remaining.saturating_sub(now.elapsed());
            update_sender.send(remaining).unwrap();
          }

          if remaining.is_zero() {
            let (lock, _) = &*running_state;
            *lock.lock().unwrap() = false;
          }
        }
      }
    });

    Timer {
      reset_sender,
      running_state,
      update_receiver,
    }
  }

  fn set_running_state(&self, new_state: bool) {
    let (lock, cvar) = &*self.running_state;
    // TODO: Figure out how to return error here instead of unwrapping
    let mut is_running = lock.lock().unwrap();
    *is_running = new_state;
    cvar.notify_one();
  }

  pub fn is_running(&self) -> bool {
    *self.running_state.0.lock().unwrap()
  }

  pub fn pause(&self) {
    self.set_running_state(false)
  }

  pub fn resume(&self) {
    self.reset_sender.send(None).unwrap();
    self.set_running_state(true)
  }

  pub fn toggle(&self) {
    if self.is_running() {
      self.pause()
    } else {
      self.resume()
    }
  }

  pub fn reset(&self, new_duration: Duration) -> Result<(), SendError<Option<Duration>>> {
    self.pause();
    self.reset_sender.send(Some(new_duration))
  }
}

impl Drop for Timer {
  fn drop(&mut self) {
    // Stopping timer if it's started to join timer thread faster
    self.pause();
  }
}

#[cfg(test)]
mod tests {

  use crossbeam_channel::RecvTimeoutError;

  use super::*;

  #[test]
  fn it_initializes_with_stopped_timer() {
    let update_frequency = Duration::from_millis(1);
    let timer = Timer::new(update_frequency);

    assert!(!timer.is_running());
    assert_eq!(
      // Doubling timeout due to condvar.wait_timeout not being exact
      timer.update_receiver.recv_timeout(update_frequency * 2),
      Err(RecvTimeoutError::Timeout)
    );
  }

  #[test]
  fn it_runs_once_during_update_frequency() {
    let update_frequency = Duration::from_millis(1);
    let timer = Timer::new(update_frequency);

    timer.reset(Duration::from_secs(1)).unwrap();
    timer.resume();

    assert!(timer.is_running());
    assert!(timer
      .update_receiver
      // Doubling timeout due to condvar.wait_timeout not being exact
      .recv_timeout(update_frequency * 2)
      .is_ok());
  }

  #[test]
  fn it_stops_when_time_has_ended() {
    let update_frequency = Duration::from_millis(1);
    let timer = Timer::new(update_frequency);

    timer.reset(update_frequency).unwrap();
    timer.resume();

    timer.update_receiver.recv().unwrap();
    timer.update_receiver.recv().unwrap();

    assert!(!timer.is_running());
  }

  #[test]
  fn it_pauses_timer() {
    let update_frequency = Duration::from_millis(100);
    let timer = Timer::new(update_frequency);

    timer.reset(update_frequency).unwrap();
    timer.resume();
    timer.pause();

    assert!(!timer.is_running());
  }

  #[test]
  fn it_pauses_timer_on_reset() {
    let timer = Timer::new(Duration::ZERO);

    timer.reset(Duration::from_secs(1)).unwrap();

    assert!(!timer.is_running());
  }
}
