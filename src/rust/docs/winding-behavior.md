# Winding Behavior (Rust Port)

This document describes the winding routine as implemented in the Rust core
(`src/rust/osww-firmware/src/controller.rs`). The goal is to mirror the
original C++ firmware behavior while keeping all time/motor logic testable.

## Key Settings
- **rotations_per_day**: total rotations required per day.
- **rotation_duration_secs**: seconds to complete a single rotation.
- **custom_wind_duration_secs**: how long to run before pausing.
- **custom_wind_pause_secs**: how long to pause.
- **direction**: `CW`, `CCW`, or `BOTH`.

## Estimated Routine Duration
The total routine duration is computed as:

```
turning = rotations_per_day * rotation_duration_secs
rest_periods = turning / custom_wind_duration_secs
rest = rest_periods * custom_wind_pause_secs
estimated_duration = turning + rest
```

In Rust we guard against a `custom_wind_duration_secs == 0` by treating it as
**"no rest"** (estimated duration = turning). This avoids division-by-zero for
invalid configs while keeping normal behavior identical.

## Start Conditions
A routine starts when:
- timer is enabled, AND
- current time-of-day equals the configured timer start time, AND
- no routine is running, AND
- winder is enabled.

Starting a routine sets:
- `start_epoch` and `previous_epoch` to the current epoch
- `status = Winding`
- `estimated_finish_epoch = start + estimated_duration`

## Main Winding Loop
While `routine.running` and `now_epoch < estimated_finish_epoch`:

1. **Motor runs** in the current motor direction.
2. **Random sampling gate**: ~25% of ticks (`rand % 100 <= 25`) check whether
   it’s time to pause. This mirrors the original C++ loop and adds small
   jitter to pause boundaries.
3. If `elapsed_since(previous_epoch) > custom_wind_duration_secs`:
   - stop the motor
   - pause for `custom_wind_pause_secs`
   - update `previous_epoch` to `now + pause`
   - if `direction == BOTH`, toggle motor direction
   - restart the motor

## Routine Completion
When `now_epoch >= estimated_finish_epoch`:
- `status = Stopped`
- `routine.running = false`
- motor is stopped
- settings snapshot is persisted

## Cycle Progress
Progress is computed every tick as:

```
progress = (now - start_epoch) / (estimated_finish_epoch - start_epoch)
```

The value is clamped to `[0.0, 1.0]`. When no routine is running, progress is
reset to `0.0`.

## Tests (Winding Behavior)
The winding algorithm is covered in unit tests under:
- `src/rust/osww-firmware/src/controller.rs`

Key tests include:
- **duration math**: matches the C++ formula
- **timer start**: begins a routine at the configured time
- **pause behavior**: only triggers when the random gate fires and the elapsed
  time exceeds `custom_wind_duration_secs`
- **direction toggling**: only in `BOTH` mode
- **finish behavior**: routine stops and persists settings

These tests are designed to keep the logic independent of hardware IO.
