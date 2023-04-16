use std::{
    sync::{mpsc, Arc, Condvar, Mutex},
    thread,
    time::{Duration, Instant},
};

pub struct TimerSettings {}

pub struct Timer {
    /** Restart/reset timer, if duration is not specified will continue using remaining duration */
    reset_sender: mpsc::Sender<Option<Duration>>,
    /** Contains isRunning boolean mutex with Condvar for notifying when value changes */
    running_state: Arc<(Mutex<bool>, Condvar)>,
}

impl Timer {
    pub fn new<UpdateFn: Fn(Duration) + Send + Sync + 'static>(
        update_fn: UpdateFn,
        update_frequency: Duration,
    ) -> Self {
        let running_state = Arc::new((Mutex::new(false), Condvar::new()));
        let running_state_clone = running_state.clone();
        let (reset_sender, reset_receiver) = mpsc::channel::<Option<Duration>>();
        let mut remaining = Duration::ZERO;

        thread::spawn(move || {
            while let Ok(reset_payload) = reset_receiver.recv() {
                remaining = reset_payload.unwrap_or(remaining);
                let (lock, cvar) = &*running_state_clone;

                while !remaining.is_zero() {
                    let now = Instant::now();

                    let (is_running, _) = cvar
                        .wait_timeout(lock.lock().unwrap(), update_frequency)
                        .unwrap();

                    remaining = remaining.saturating_sub(now.elapsed());
                    update_fn(remaining);

                    if !*is_running {
                        return;
                    }
                }

                if remaining.is_zero() {
                    let (lock, _) = &*running_state_clone;
                    *lock.lock().unwrap() = false;
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
        self.set_running_state(true)
    }

    pub fn toggle(&self) {
        if self.is_running() {
            self.pause()
        } else {
            self.resume()
        }
    }

    pub fn reset(&self, new_duration: Duration) -> Result<(), mpsc::SendError<Option<Duration>>> {
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
    use std::{
        sync::atomic::{AtomicBool, Ordering},
        thread::sleep,
    };

    use super::*;

    #[test]
    fn it_initializes_with_stopped_timer() {
        let update_frequency = Duration::from_millis(100);
        let is_called = Arc::new(AtomicBool::new(false));
        let is_called_clone = is_called.clone();
        let timer = Timer::new(
            move |_| {
                is_called_clone.store(true, Ordering::SeqCst);
            },
            update_frequency,
        );

        assert!(!timer.is_running());

        sleep(update_frequency + update_frequency);

        assert!(!is_called.load(Ordering::SeqCst));
    }

    #[test]
    fn it_runs_once_during_update_frequency() {
        let update_frequency = Duration::from_millis(100);
        let is_called = Arc::new(AtomicBool::new(false));
        let is_called_clone = is_called.clone();
        let timer = Timer::new(
            move |_| {
                is_called_clone.store(true, Ordering::SeqCst);
            },
            update_frequency,
        );

        timer.reset(Duration::from_secs(1)).unwrap();
        timer.resume();

        assert!(timer.is_running());

        sleep(update_frequency + update_frequency);

        assert!(is_called.load(Ordering::SeqCst));
    }

    #[test]
    fn it_stops_when_time_has_ended() {
        let update_frequency = Duration::from_millis(100);
        let timer = Timer::new(|_| {}, update_frequency);

        timer.reset(update_frequency).unwrap();
        timer.resume();

        sleep(update_frequency + update_frequency);

        assert!(!timer.is_running());
    }

    #[test]
    fn it_pauses_timer() {
        let update_frequency = Duration::from_millis(100);
        let timer = Timer::new(|_| {}, update_frequency);

        timer.reset(update_frequency).unwrap();
        timer.resume();
        timer.pause();

        assert!(!timer.is_running());
    }

    #[test]
    fn it_pauses_timer_on_reset() {
        let timer = Timer::new(|_| {}, Duration::ZERO);

        timer.reset(Duration::from_secs(1)).unwrap();

        assert!(!timer.is_running());
    }
}
