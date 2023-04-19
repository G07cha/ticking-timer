# Ticking timer

A timer that emits channel notifications at a regular (imprecise) interval.
Emitted notifications contain remaining duration and can be received synchronously in a separate thread.

## Examples

```rust
use std::time::Duration;
use ticking_timer::Timer;

let timer = Timer::new(Duration::from_millis(100));
timer.reset(Duration::from_millis(1000));
timer.resume();

timer.update_receiver.recv().unwrap();
let value = timer.update_receiver.recv().unwrap();
assert!(value.as_millis().le(&900));
```

## License

The project is licensed under the [MIT License](./LICENSE).
