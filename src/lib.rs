//! A Ticking Timer
//!
//! A timer that emits channel notifications at a regular (imprecise) interval.
//! Emitted notifications contain remaining duration and can be received synchronously in a separate thread.
//!
//! # Examples
//!
//! ```
//! use std::time::Duration;
//! use std::thread;
//! use ticking_timer::Timer;
//!
//! let timer = Timer::new(Duration::from_millis(100), |remaining| {
//!   println!("{} milliseconds remaining", remaining.as_millis());
//! });
//!
//! timer.reset(Duration::from_millis(1000));
//! timer.resume();
//! ```
#![warn(clippy::unwrap_used)]

use crossbeam_channel::{bounded, SendError, Sender};
use std::{
  sync::{Arc, Condvar, Mutex, MutexGuard},
  thread,
  time::{Duration, SystemTime},
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TimerError {
  #[error("thread communication error {0}")]
  ThreadCommunication(#[from] SendError<Option<Duration>>),
}

pub(crate) type Result<T> = std::result::Result<T, TimerError>;

pub struct Timer {
  /** Restart/reset timer, if duration is not specified will continue using remaining duration */
  reset_sender: Sender<Option<Duration>>,
  /** Contains isRunning boolean mutex with Condvar for notifying when value changes */
  running_state: Arc<(Mutex<bool>, Condvar)>,
}

impl Timer {
  /// Creates a new [`Timer`].
  /// # Examples
  ///
  /// ```
  /// use std::time::Duration;
  /// use ticking_timer::Timer;
  ///
  /// let timer = Timer::new(Duration::from_millis(100), |remaining| println!("{} seconds remaining", remaining.as_secs()));
  /// ```
  ///
  /// # Panics
  ///
  /// Panics if unable to spawn a separate thread for tracking time and emitting ticks.
  pub fn new<F>(update_frequency: Duration, callback: F) -> Self
  where
    F: 'static + Fn(Duration) + Send + Sync,
  {
    let running_state = Arc::new((Mutex::new(false), Condvar::new()));
    let (reset_sender, reset_receiver) = bounded::<Option<Duration>>(0);
    let mut remaining = Duration::ZERO;

    thread::spawn({
      let running_state = running_state.clone();
      move || {
        while let Ok(reset_payload) = reset_receiver.recv() {
          remaining = reset_payload.unwrap_or(remaining);
          let (lock, cvar) = &*running_state;

          while !remaining.is_zero() {
            let now = SystemTime::now();

            let (is_running, _) = cvar
              .wait_timeout(
                lock.lock().expect("Unable to acquire a lock"),
                update_frequency,
              )
              .expect("Failed to wait for timeout");

            if !*is_running {
              break;
            }

            remaining = remaining.saturating_sub(now.elapsed().expect("Cannot get system time"));
            callback(remaining);
          }

          if remaining.is_zero() {
            let (lock, _) = &*running_state;
            *lock.lock().unwrap_or_else(|poison| poison.into_inner()) = false;
          }
        }
      }
    });

    Timer {
      reset_sender,
      running_state,
    }
  }

  fn set_running_state(&self, new_state: bool) {
    let (lock, cvar) = &*self.running_state;
    let mut is_running = lock.lock().unwrap_or_else(|poison| poison.into_inner());
    *is_running = new_state;
    cvar.notify_one();
  }

  /// Returns the is running state of this [`Timer`].
  pub fn is_running(&self) -> MutexGuard<bool> {
    self
      .running_state
      .0
      .lock()
      .unwrap_or_else(|poison| poison.into_inner())
  }

  /// Pauses this [`Timer`]. Does nothing if [`Timer`] is already paused.
  pub fn pause(&self) {
    self.set_running_state(false)
  }

  /// Resumes this [`Timer`]. Does nothing if [`Timer`] is already running.
  pub fn resume(&self) -> Result<()> {
    self.reset_sender.send(None)?;
    self.set_running_state(true);
    Ok(())
  }

  /// Switches running state of this [`Timer`] from running to paused and vice versa.
  pub fn toggle(&self) -> Result<()> {
    if *self.is_running() {
      self.pause();
    } else {
      self.resume()?;
    }

    Ok(())
  }

  /// Pauses and resets this [`Timer`] to a new duration.
  ///
  /// # Examples
  ///
  /// ```
  /// use std::time::Duration;
  /// use ticking_timer::Timer;
  ///
  /// let timer = Timer::new(Duration::from_millis(100), |_| {});
  /// timer.reset(Duration::from_secs(1));
  /// timer.resume();
  /// assert!(*timer.is_running());
  ///
  /// timer.reset(Duration::ZERO);
  /// assert!(!*timer.is_running());
  /// ```
  ///
  /// # Errors
  ///
  /// This function will return an error if unable to communicate with time-tracking thread.
  pub fn reset(&self, new_duration: Duration) -> Result<()> {
    self.pause();
    self.reset_sender.send(Some(new_duration))?;
    Ok(())
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

  use std::{
    sync::atomic::{AtomicBool, Ordering},
    thread::sleep,
  };

  use super::*;

  #[test]
  fn it_initializes_with_stopped_timer() {
    let update_frequency = Duration::from_millis(100);
    let is_called = Arc::new(AtomicBool::new(false));
    let timer = Timer::new(update_frequency, {
      let is_called = is_called.clone();
      move |_| is_called.store(true, Ordering::Relaxed)
    });

    assert!(!*timer.is_running());

    sleep(update_frequency * 2);

    assert!(!is_called.load(Ordering::Relaxed));
  }

  #[test]
  fn it_stops_when_time_has_ended() -> super::Result<()> {
    let update_frequency = Duration::from_millis(100);
    let timer = Timer::new(update_frequency, |_| {});

    timer.reset(update_frequency)?;
    timer.resume()?;

    sleep(update_frequency * 2);

    assert!(!*timer.is_running());

    Ok(())
  }

  #[test]
  fn it_pauses_timer() -> super::Result<()> {
    let update_frequency = Duration::from_millis(100);
    let timer = Timer::new(update_frequency, |_| {});

    timer.reset(update_frequency)?;
    timer.resume()?;
    timer.pause();

    assert!(!*timer.is_running());

    Ok(())
  }

  #[test]
  fn it_pauses_timer_on_reset() -> super::Result<()> {
    let timer = Timer::new(Duration::ZERO, |_| {});

    timer.reset(Duration::from_secs(1))?;

    assert!(!*timer.is_running());

    Ok(())
  }

  #[test]
  fn it_toggles_timer() -> super::Result<()> {
    let timer = Timer::new(Duration::from_millis(100), |_| {});

    timer.reset(Duration::from_secs(1))?;
    timer.toggle()?;

    assert!(*timer.is_running());

    timer.toggle()?;
    assert!(!*timer.is_running());

    Ok(())
  }
}
