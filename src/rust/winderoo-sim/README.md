# Winderoo Simulator (Dioxus)

Host-side GUI simulator for the `winderoo-firmware` controller logic.

## Run

From the repo root:

```bash
cargo run --manifest-path "src/rust/winderoo-sim/Cargo.toml"
```

## What it simulates

- The real `winderoo-firmware::controller::Controller` state machine
- A virtual clock (`tick=500ms` by default)
- Simulated IO surfaces driven by `ControllerEvent`s:
  - motor start/stop + direction
  - display clear/static/dynamic/notifications
  - LED patterns
  - settings persistence + restart requests

## Tips

- **Run/Pause** toggles continuous ticking.
- **Steps/frame** controls sim speed (each step advances by one firmware tick).
- **Update payload** exercises the same `apply_update` path as `/api/update`.

