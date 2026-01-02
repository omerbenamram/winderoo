# Winderoo Web Simulator

A beautiful 3D web-based simulator for the Winderoo watch winder, featuring:

- **3D Watch Winder Rendering** - Three.js powered visualization of the actual device
- **OLED Display Emulation** - Pixel-perfect recreation of the device's OLED screen
- **Algorithm Trace** - Real-time visualization of the winding state machine
- **Rust WASM Integration** - Your actual firmware logic compiled to WebAssembly

## Screenshot

The simulator displays:
- Left: Interactive 3D model of the watch winder with rotating drum
- Center: Emulated OLED display showing status, progress, and notifications
- Right: Algorithm trace showing motor events, pauses, and state transitions

## Getting Started

### Prerequisites

- [Rust](https://rustup.rs/) with `wasm32-unknown-unknown` target
- [wasm-pack](https://rustwasm.github.io/wasm-pack/installer/)
- [Node.js](https://nodejs.org/) >= 18

### Install wasm-pack

```bash
cargo install wasm-pack
```

### Add WASM target

```bash
rustup target add wasm32-unknown-unknown
```

### Build and Run

```bash
# Build the WASM module
wasm-pack build --target web

# Install dependencies
npm install

# Start development server
npm run dev
```

Then open http://localhost:5173 in your browser.

### Production Build

```bash
npm run build
```

## Architecture

```
winderoo-sim-web/
├── src/
│   ├── lib.rs          # WASM bindings wrapping winderoo-firmware
│   └── main.ts         # Three.js scene and UI logic
├── pkg/                # Generated WASM package (after build)
├── index.html          # Main HTML with UI structure
├── Cargo.toml          # Rust dependencies
└── package.json        # Node dependencies
```

## Features

### 3D Visualization
- Realistic watch winder model based on the actual hardware
- Smooth drum rotation animation synced with motor state
- Dynamic lighting and shadows
- Interactive camera (drag to orbit, scroll to zoom)

### OLED Display
- Authentic OLED green glow effect
- Shows status, direction, TPD, and progress
- Notification system with pulse animation

### Algorithm Trace
- Real-time event log (Motor Start/Stop, Pauses, Display updates)
- Color-coded events for quick scanning
- Shows the firmware state machine in action

### Controls
- Start/Stop winding
- Direction selection (CW/CCW/Both)
- TPD (Turns Per Day) configuration
- Wind duration and pause settings
- Timer configuration
- Simulation speed control

## Development

The simulator uses the same `winderoo-firmware` crate as the real device, compiled to WebAssembly. This ensures the simulation accurately reflects the actual firmware behavior.

### Key Components

- `WasmSimulator` - Rust struct exposed to JS via wasm-bindgen
- `TracedEvent` - Events emitted by the controller for visualization
- `SimState` - Current state snapshot for UI updates

### Adding New Features

1. Add functionality to `winderoo-firmware` crate
2. Expose it through `WasmSimulator` in `src/lib.rs`
3. Update `main.ts` to use the new functionality
