import * as THREE from 'three';
import type { WasmSimulator as WasmSimulatorWasm } from '../pkg/winderoo_sim_web';

// Types for the WASM simulator
interface SimState {
  now_ms: number;
  now_epoch: number;
  time_of_day: string;
  motor_running: boolean;
  motor_direction: string | null;
  motor_angle: number;
  led_pattern: string | null;
  display_on: boolean;
  display_title: string | null;
  display_notification: string | null;
  winder_enabled: boolean;
  status: string;
  direction: string;
  rotations_per_day: number;
  timer_enabled: boolean;
  timer_time: string;
  cycle_progress: number;
  routine_running: boolean;
  routine_start_epoch: number;
  routine_finish_epoch: number;
  custom_wind_duration_secs: number;
  custom_wind_pause_secs: number;
  rotation_duration_secs: number;
  gmt_offset: number;
  dst: boolean;
  screen_schedule_enabled: boolean;
  screen_schedule_start: string;
  screen_schedule_end: string;
  screen_sleep: boolean;
}

interface TracedEvent {
  type: string;
  count?: number;
  direction?: string;
  seconds?: number;
  title?: string;
  message?: string;
  pattern?: string;
  epoch?: number;
}

// App state
let simulator: WasmSimulatorWasm | null = null;
let running = true;
let speed = 1;
let lastTime = 0;
let lastSimState: SimState | null = null;

// Three.js scene elements
let scene: THREE.Scene;
let camera: THREE.PerspectiveCamera;
let renderer: THREE.WebGLRenderer;
let watchWinder: THREE.Group;
let rotatingParts: THREE.Group; // Group for all rotating parts
let statusLED: THREE.Mesh;
let oledLight: THREE.PointLight;
let motorIndicator: THREE.Mesh; // Visual indicator for rotation

// Initialize everything
async function init() {
  initThreeJS();
  initUI();
  await initWasm();
  animate();
}

// Load WASM module
async function initWasm() {
  try {
    const wasm = await import('../pkg/winderoo_sim_web.js');
    await wasm.default();
    simulator = new wasm.WasmSimulator();
    console.log('WASM simulator initialized');
    updateUI();
  } catch (e) {
    console.error('Failed to load WASM:', e);
    // Show error in trace view
    const traceView = document.getElementById('trace-view')!;
    traceView.innerHTML = `
      <div style="padding: 20px; color: var(--danger);">
        <p><strong>WASM not built yet!</strong></p>
        <p style="margin-top: 10px; font-size: 12px; color: var(--text-secondary);">
          Run the following commands:
        </p>
        <pre style="margin-top: 10px; padding: 12px; background: var(--bg-tertiary); border-radius: 8px; font-size: 11px;">
cd src/rust/winderoo-sim-web
wasm-pack build --target web
npm install
npm run dev</pre>
      </div>
    `;
  }
}

// Initialize Three.js scene
function initThreeJS() {
  const canvas = document.getElementById('three-canvas') as HTMLCanvasElement;
  const container = canvas.parentElement!;

  // Scene - slightly lighter background
  scene = new THREE.Scene();
  scene.background = new THREE.Color(0x12121a);

  // Camera
  camera = new THREE.PerspectiveCamera(
    45,
    container.clientWidth / container.clientHeight,
    0.1,
    1000
  );
  camera.position.set(4, 3, 5);
  camera.lookAt(0, 0, 0);

  // Renderer
  renderer = new THREE.WebGLRenderer({ canvas, antialias: true });
  renderer.setSize(container.clientWidth, container.clientHeight);
  renderer.setPixelRatio(window.devicePixelRatio);
  renderer.shadowMap.enabled = true;
  renderer.shadowMap.type = THREE.PCFSoftShadowMap;
  renderer.toneMapping = THREE.ACESFilmicToneMapping;
  renderer.toneMappingExposure = 1.8;

  // Lighting - much brighter for visibility
  const ambientLight = new THREE.AmbientLight(0xffffff, 0.6);
  scene.add(ambientLight);

  const keyLight = new THREE.DirectionalLight(0xffffff, 1.5);
  keyLight.position.set(5, 8, 8);
  keyLight.castShadow = true;
  keyLight.shadow.mapSize.width = 2048;
  keyLight.shadow.mapSize.height = 2048;
  keyLight.shadow.camera.near = 0.5;
  keyLight.shadow.camera.far = 50;
  scene.add(keyLight);

  const fillLight = new THREE.DirectionalLight(0x88aaff, 0.8);
  fillLight.position.set(-5, 5, 5);
  scene.add(fillLight);

  const backLight = new THREE.DirectionalLight(0xffffff, 0.5);
  backLight.position.set(0, 3, -5);
  scene.add(backLight);

  const bottomLight = new THREE.DirectionalLight(0x6366f1, 0.4);
  bottomLight.position.set(0, -3, 5);
  scene.add(bottomLight);

  // OLED glow light (will be toggled)
  oledLight = new THREE.PointLight(0x00ffaa, 0.5, 3);
  oledLight.position.set(0, -0.8, 0.6);
  scene.add(oledLight);

  // Create the watch winder
  createWatchWinder();

  // Ground plane with better visibility
  const groundGeometry = new THREE.PlaneGeometry(20, 20);
  const groundMaterial = new THREE.MeshStandardMaterial({
    color: 0x15151a,
    roughness: 0.85,
  });
  const ground = new THREE.Mesh(groundGeometry, groundMaterial);
  ground.rotation.x = -Math.PI / 2;
  ground.position.y = -1.5;
  ground.receiveShadow = true;
  scene.add(ground);

  // Grid helper - more visible
  const grid = new THREE.GridHelper(10, 20, 0x3a3a4a, 0x252530);
  grid.position.y = -1.49;
  scene.add(grid);

  // Handle resize
  window.addEventListener('resize', () => {
    camera.aspect = container.clientWidth / container.clientHeight;
    camera.updateProjectionMatrix();
    renderer.setSize(container.clientWidth, container.clientHeight);
  });

  // Mouse orbit controls (simple version)
  let isDragging = false;
  let previousMousePosition = { x: 0, y: 0 };
  let theta = Math.PI / 4;
  let phi = Math.PI / 6;
  let radius = 7;

  canvas.addEventListener('mousedown', (e) => {
    isDragging = true;
    previousMousePosition = { x: e.clientX, y: e.clientY };
  });

  canvas.addEventListener('mousemove', (e) => {
    if (!isDragging) return;

    const deltaX = e.clientX - previousMousePosition.x;
    const deltaY = e.clientY - previousMousePosition.y;

    theta -= deltaX * 0.01;
    phi = Math.max(0.1, Math.min(Math.PI / 2 - 0.1, phi + deltaY * 0.01));

    updateCameraPosition();
    previousMousePosition = { x: e.clientX, y: e.clientY };
  });

  canvas.addEventListener('mouseup', () => {
    isDragging = false;
  });

  canvas.addEventListener('wheel', (e) => {
    radius = Math.max(3, Math.min(15, radius + e.deltaY * 0.01));
    updateCameraPosition();
  });

  function updateCameraPosition() {
    camera.position.x = radius * Math.sin(theta) * Math.cos(phi);
    camera.position.y = radius * Math.sin(phi);
    camera.position.z = radius * Math.cos(theta) * Math.cos(phi);
    camera.lookAt(0, 0, 0);
  }

  // Camera preset buttons
  document.getElementById('btn-cam-front')?.addEventListener('click', () => {
    theta = 0; phi = 0.2; radius = 6;
    updateCameraPosition();
  });

  document.getElementById('btn-cam-side')?.addEventListener('click', () => {
    theta = Math.PI / 2; phi = 0.2; radius = 6;
    updateCameraPosition();
  });

  document.getElementById('btn-cam-top')?.addEventListener('click', () => {
    theta = 0; phi = Math.PI / 2.5; radius = 8;
    updateCameraPosition();
  });
}

// Create the watch winder 3D model
function createWatchWinder() {
  watchWinder = new THREE.Group();

  // Materials - brighter colors for better visibility
  const bodyMaterial = new THREE.MeshStandardMaterial({
    color: 0x2d2d35,
    roughness: 0.6,
    metalness: 0.2,
  });

  const screwMaterial = new THREE.MeshStandardMaterial({
    color: 0x555555,
    roughness: 0.3,
    metalness: 0.9,
  });

  const cushionMaterial = new THREE.MeshStandardMaterial({
    color: 0x1a1a22,
    roughness: 0.95,
    metalness: 0,
  });

  const oledFrameMaterial = new THREE.MeshStandardMaterial({
    color: 0x151515,
    roughness: 0.7,
    metalness: 0.2,
  });

  const ledMaterial = new THREE.MeshStandardMaterial({
    color: 0x222222,
    roughness: 0.4,
    metalness: 0.5,
    emissive: 0x000000,
  });

  const innerRingMaterial = new THREE.MeshStandardMaterial({
    color: 0x0f0f12,
    roughness: 0.9,
    side: THREE.DoubleSide,
  });

  // Main body (rounded box approximation)
  const bodySize = 2.2;
  const bodyDepth = 1.8;
  const bodyGeometry = new THREE.BoxGeometry(bodySize, bodySize, bodyDepth);
  // Round the edges slightly by using bevel
  const body = new THREE.Mesh(bodyGeometry, bodyMaterial);
  body.castShadow = true;
  body.receiveShadow = true;
  watchWinder.add(body);

  // Inner circular cutout visual (dark ring)
  const ringGeometry = new THREE.RingGeometry(0.7, 0.9, 32);
  const innerRing = new THREE.Mesh(ringGeometry, innerRingMaterial);
  innerRing.position.z = bodyDepth / 2 + 0.01;
  watchWinder.add(innerRing);

  // Create a group for all rotating parts (drum, cushion, watch)
  rotatingParts = new THREE.Group();
  rotatingParts.position.z = bodyDepth / 2 - 0.2;
  watchWinder.add(rotatingParts);

  // Drum that holds the watch cushion
  const drumGeometry = new THREE.CylinderGeometry(0.65, 0.65, 0.8, 32);
  const drum = new THREE.Mesh(drumGeometry, bodyMaterial);
  drum.rotation.x = Math.PI / 2;
  drum.position.z = -0.2;
  drum.castShadow = true;
  rotatingParts.add(drum);

  // Add rotation indicator stripe on drum (so you can see it spinning)
  const stripeGeometry = new THREE.BoxGeometry(0.08, 0.82, 0.02);
  const stripeMaterial = new THREE.MeshStandardMaterial({
    color: 0x00ffaa,
    emissive: 0x00ffaa,
    emissiveIntensity: 0.3,
  });
  motorIndicator = new THREE.Mesh(stripeGeometry, stripeMaterial);
  motorIndicator.position.set(0.6, 0, -0.2);
  rotatingParts.add(motorIndicator);

  // Second indicator on opposite side
  const stripe2 = new THREE.Mesh(stripeGeometry, stripeMaterial);
  stripe2.position.set(-0.6, 0, -0.2);
  rotatingParts.add(stripe2);

  // Watch cushion (pillow shape)
  const cushionGeometry = new THREE.CapsuleGeometry(0.35, 0.6, 8, 16);
  const watchCushion = new THREE.Mesh(cushionGeometry, cushionMaterial);
  watchCushion.rotation.z = Math.PI / 2;
  watchCushion.position.z = 0.2;
  watchCushion.castShadow = true;
  rotatingParts.add(watchCushion);

  // Simulated watch on cushion
  const watchGroup = new THREE.Group();

  // Watch case
  const watchCaseGeometry = new THREE.CylinderGeometry(0.22, 0.22, 0.08, 32);
  const watchCaseMaterial = new THREE.MeshStandardMaterial({
    color: 0xc0c0c0,
    roughness: 0.2,
    metalness: 0.9,
  });
  const watchCase = new THREE.Mesh(watchCaseGeometry, watchCaseMaterial);
  watchCase.rotation.x = Math.PI / 2;
  watchGroup.add(watchCase);

  // Watch dial
  const dialGeometry = new THREE.CircleGeometry(0.18, 32);
  const dialMaterial = new THREE.MeshStandardMaterial({
    color: 0xd4af37, // Gold
    roughness: 0.3,
    metalness: 0.7,
  });
  const dial = new THREE.Mesh(dialGeometry, dialMaterial);
  dial.position.z = 0.041;
  watchGroup.add(dial);

  // Watch band (simplified)
  const bandGeometry = new THREE.BoxGeometry(0.15, 0.5, 0.02);
  const bandMaterial = new THREE.MeshStandardMaterial({
    color: 0x888888,
    roughness: 0.3,
    metalness: 0.8,
  });
  const bandTop = new THREE.Mesh(bandGeometry, bandMaterial);
  bandTop.position.y = 0.35;
  watchGroup.add(bandTop);
  const bandBottom = new THREE.Mesh(bandGeometry, bandMaterial);
  bandBottom.position.y = -0.35;
  watchGroup.add(bandBottom);

  watchGroup.position.z = 0.25;
  rotatingParts.add(watchGroup);

  // Corner screws
  const screwGeometry = new THREE.CylinderGeometry(0.06, 0.06, 0.05, 6);
  const screwPositions = [
    [-bodySize / 2 + 0.15, bodySize / 2 - 0.15],
    [bodySize / 2 - 0.15, bodySize / 2 - 0.15],
    [-bodySize / 2 + 0.15, -bodySize / 2 + 0.15],
    [bodySize / 2 - 0.15, -bodySize / 2 + 0.15],
  ];

  screwPositions.forEach(([x, y]) => {
    const screw = new THREE.Mesh(screwGeometry, screwMaterial);
    screw.position.set(x, y, bodyDepth / 2 + 0.025);
    screw.rotation.x = Math.PI / 2;
    watchWinder.add(screw);
  });

  // OLED display area (bottom)
  const oledGeometry = new THREE.BoxGeometry(1.2, 0.5, 0.1);
  const oledFrame = new THREE.Mesh(oledGeometry, oledFrameMaterial);
  oledFrame.position.set(0, -bodySize / 2 + 0.35, bodyDepth / 2 + 0.05);
  watchWinder.add(oledFrame);

  // OLED screen glow
  const oledScreenGeometry = new THREE.PlaneGeometry(1.0, 0.35);
  const oledScreenMaterial = new THREE.MeshBasicMaterial({
    color: 0x00ffaa,
    transparent: true,
    opacity: 0.8,
  });
  const oledScreen = new THREE.Mesh(oledScreenGeometry, oledScreenMaterial);
  oledScreen.position.set(0, -bodySize / 2 + 0.35, bodyDepth / 2 + 0.11);
  oledScreen.name = 'oledScreen';
  watchWinder.add(oledScreen);

  // Status LED
  const ledGeometry = new THREE.SphereGeometry(0.04, 16, 16);
  statusLED = new THREE.Mesh(ledGeometry, ledMaterial);
  statusLED.position.set(0.5, -bodySize / 2 + 0.35, bodyDepth / 2 + 0.15);
  statusLED.name = 'statusLED';
  watchWinder.add(statusLED);

  // Stand feet
  const footGeometry = new THREE.BoxGeometry(0.3, 0.08, 0.4);
  const footMaterial = new THREE.MeshStandardMaterial({
    color: 0x2a2a32,
    roughness: 0.7,
  });

  const footLeft = new THREE.Mesh(footGeometry, footMaterial);
  footLeft.position.set(-0.6, -bodySize / 2 - 0.04, bodyDepth / 2 + 0.3);
  footLeft.castShadow = true;
  watchWinder.add(footLeft);

  const footRight = new THREE.Mesh(footGeometry, footMaterial);
  footRight.position.set(0.6, -bodySize / 2 - 0.04, bodyDepth / 2 + 0.3);
  footRight.castShadow = true;
  watchWinder.add(footRight);

  // Position the whole winder
  watchWinder.rotation.x = -0.2; // Tilt back slightly
  scene.add(watchWinder);
}

// Initialize UI controls
function initUI() {
  // Simulation controls
  document.getElementById('btn-pause')?.addEventListener('click', () => {
    running = !running;
    const btn = document.getElementById('btn-pause')!;
    btn.innerHTML = running ? '<span>⏸</span> Pause' : '<span>▶</span> Play';
  });

  document.getElementById('btn-step')?.addEventListener('click', () => {
    if (simulator) {
      simulator.stepTick();
      updateUI();
    }
  });

  document.getElementById('btn-reset')?.addEventListener('click', () => {
    if (simulator) {
      simulator.resetSim();
      updateUI();
    }
  });

  // Action buttons
  document.getElementById('btn-start')?.addEventListener('click', () => {
    if (simulator) {
      simulator.start();
      updateUI();
    }
  });

  document.getElementById('btn-stop')?.addEventListener('click', () => {
    if (simulator) {
      simulator.stop();
      updateUI();
    }
  });

  // Settings controls
  document.getElementById('ctrl-direction')?.addEventListener('change', (e) => {
    if (simulator) {
      simulator.setDirection((e.target as HTMLSelectElement).value);
    }
  });

  document.getElementById('ctrl-tpd')?.addEventListener('change', (e) => {
    if (simulator) {
      simulator.setRotationsPerDay(parseInt((e.target as HTMLInputElement).value));
    }
  });

  document.getElementById('ctrl-wind-duration')?.addEventListener('change', (e) => {
    if (simulator) {
      simulator.setWindDuration(parseInt((e.target as HTMLInputElement).value));
    }
  });

  document.getElementById('ctrl-wind-pause')?.addEventListener('change', (e) => {
    if (simulator) {
      simulator.setWindPause(parseInt((e.target as HTMLInputElement).value));
    }
  });

  document.getElementById('ctrl-timer-enabled')?.addEventListener('change', (e) => {
    if (simulator) {
      simulator.setTimerEnabled((e.target as HTMLInputElement).checked);
    }
  });

  document.getElementById('ctrl-timer-time')?.addEventListener('change', (e) => {
    if (simulator) {
      const [hour, minute] = (e.target as HTMLInputElement).value.split(':').map(Number);
      simulator.setTimerTime(hour, minute);
    }
  });

  document.getElementById('ctrl-power')?.addEventListener('change', (e) => {
    if (simulator) {
      simulator.setPower((e.target as HTMLInputElement).checked);
      updateUI();
    }
  });

  document.getElementById('ctrl-speed')?.addEventListener('change', (e) => {
    speed = parseFloat((e.target as HTMLSelectElement).value);
    document.getElementById('sim-speed')!.textContent = `${speed}x`;
  });

  document.getElementById('ctrl-motor-speed')?.addEventListener('change', (e) => {
    if (simulator) {
      simulator.setMotorSpeed(parseFloat((e.target as HTMLInputElement).value));
    }
  });

  // Panel tabs
  document.querySelectorAll('.panel-tab').forEach((tab) => {
    tab.addEventListener('click', () => {
      const panelId = tab.getAttribute('data-panel');

      // Update tabs
      document.querySelectorAll('.panel-tab').forEach((t) => {
        t.classList.remove('active');
      });
      tab.classList.add('active');

      // Update panels
      document.querySelectorAll('.panel-content').forEach((p) => {
        p.classList.remove('active');
      });
      document.getElementById(`panel-${panelId}`)?.classList.add('active');
    });
  });
}

// Update UI from simulator state
function updateUI() {
  if (!simulator) return;

  const state = simulator.getState() as unknown as SimState;
  lastSimState = state;

  // Header time display
  document.getElementById('sim-time')!.textContent = state.time_of_day;
  document.getElementById('sim-epoch')!.textContent = state.now_epoch.toString();

  // OLED display
  const oledScreen = document.getElementById('oled-screen')!;
  if (state.display_on && !state.screen_sleep) {
    oledScreen.classList.remove('off');
    document.getElementById('oled-status')!.textContent = state.status;
    document.getElementById('oled-direction')!.textContent = state.direction;
    document.getElementById('oled-tpd')!.textContent = state.rotations_per_day.toString();

    const progress = Math.round(state.cycle_progress * 100);
    (document.getElementById('oled-progress') as HTMLElement).style.width = `${progress}%`;
    document.getElementById('oled-progress-text')!.textContent = `${progress}%`;

    const notificationEl = document.getElementById('oled-notification')!;
    if (state.display_notification) {
      notificationEl.textContent = state.display_notification;
      notificationEl.style.display = 'block';
    } else {
      notificationEl.style.display = 'none';
    }

    // Update 3D OLED glow
    oledLight.intensity = 0.5;
    const oledMesh = watchWinder.getObjectByName('oledScreen') as THREE.Mesh;
    if (oledMesh) {
      (oledMesh.material as THREE.MeshBasicMaterial).opacity = 0.8;
    }
  } else {
    oledScreen.classList.add('off');
    oledLight.intensity = 0;
    const oledMesh = watchWinder.getObjectByName('oledScreen') as THREE.Mesh;
    if (oledMesh) {
      (oledMesh.material as THREE.MeshBasicMaterial).opacity = 0.1;
    }
  }

  // Device status
  document.getElementById('status-motor')!.textContent = state.motor_running ? '🟢 Running' : 'Stopped';
  document.getElementById('status-motor')!.className = `value ${state.motor_running ? 'active' : 'inactive'}`;
  document.getElementById('status-angle')!.textContent = `${Math.round(state.motor_angle)}°`;
  document.getElementById('status-direction')!.textContent = state.motor_direction || '—';
  document.getElementById('status-led')!.textContent = state.led_pattern || 'Off';
  document.getElementById('status-power')!.textContent = state.winder_enabled ? 'On' : 'Off';
  document.getElementById('status-power')!.className = `value ${state.winder_enabled ? 'active' : 'inactive'}`;
  document.getElementById('status-timer')!.textContent = state.timer_enabled ? `On (${state.timer_time})` : 'Off';

  // Update 3D LED - make it very visible when motor is running
  const ledMat = statusLED.material as THREE.MeshStandardMaterial;
  if (state.motor_running) {
    ledMat.emissive.setHex(0x00ff00);
    ledMat.emissiveIntensity = 3;
    ledMat.color.setHex(0x00ff00);
  } else if (state.routine_running) {
    // Routine running but motor paused (during cycle pause)
    ledMat.emissive.setHex(0xffaa00);
    ledMat.emissiveIntensity = 2;
    ledMat.color.setHex(0xffaa00);
  } else if (state.led_pattern) {
    ledMat.emissive.setHex(0xff6600);
    ledMat.emissiveIntensity = 2;
    ledMat.color.setHex(0xff6600);
  } else {
    ledMat.emissive.setHex(0x000000);
    ledMat.emissiveIntensity = 0;
    ledMat.color.setHex(0x222222);
  }

  // Update motor indicator glow based on motor state
  if (motorIndicator) {
    const indicatorMat = motorIndicator.material as THREE.MeshStandardMaterial;
    if (state.motor_running) {
      indicatorMat.emissive.setHex(0x00ff00);
      indicatorMat.emissiveIntensity = 1;
    } else {
      indicatorMat.emissive.setHex(0x00ffaa);
      indicatorMat.emissiveIntensity = 0.1;
    }
  }

  // Update trace view
  updateTraceView();
}

// Update trace visualization
function updateTraceView() {
  if (!simulator) return;

  const trace = simulator.getTrace() as unknown as any[];
  const traceView = document.getElementById('trace-view')!;
  const traceHint = document.getElementById('trace-hint');

  const toPlain = (value: any): any => {
    if (value instanceof Map) {
      const obj: Record<string, any> = {};
      for (const [k, v] of value.entries()) {
        obj[String(k)] = toPlain(v);
      }
      return obj;
    }
    if (Array.isArray(value)) return value.map(toPlain);
    return value;
  };

  // Only show non-tick events. The simulator may return either raw events:
  //   { type: "MotorStart", direction: "CCW", count?: number }
  // or coalesced entries:
  //   { event: { type: ... }, count: number }
  const normalized = (Array.isArray(trace) ? (trace as any[]) : []).map((raw) => {
    const plain = toPlain(raw);
    const event: TracedEvent = (plain?.event ?? plain) as TracedEvent;
    const count: number = (plain?.count ?? event.count ?? 1) as number;
    return { event, count };
  });

  const significantEvents = normalized
    .filter(({ event }) => event?.type !== 'Tick' && event?.type !== 'DisplayDynamic')
    .slice(-120);

  if (significantEvents.length === 0) {
    traceView.innerHTML = '<div style="padding: 20px; color: var(--text-secondary); text-align: center;">No events yet. Start winding to see algorithm trace.</div>';
    if (traceHint) traceHint.textContent = 'Trace: semantic view (no events yet)';
    return;
  }

  if (traceHint) {
    traceHint.textContent = 'Trace: semantic view (DisplayDynamic hidden, duplicates coalesced)';
  }

  const html = significantEvents.map(({ event, count }: { event: TracedEvent; count: number }) => {
    let className = 'trace-entry';
    let icon = '•';
    let text = '';

    switch (event.type) {
      case 'MotorStart':
        className += ' motor-start';
        icon = '⚡';
        text = `Motor Start (${event.direction})`;
        break;
      case 'MotorStop':
        className += ' motor-stop';
        icon = '⏹';
        text = 'Motor Stop';
        break;
      case 'PauseStart':
        className += ' pause';
        icon = '⏸';
        text = `Pause ${event.seconds}s`;
        break;
      case 'PauseEnd':
        className += ' pause';
        icon = '▶';
        text = 'Pause End';
        break;
      case 'DisplayStatic':
        className += ' display';
        icon = '📺';
        text = `Display: ${event.title}`;
        break;
      case 'DisplayNotification':
        className += ' display';
        icon = '💬';
        text = `Notify: ${event.message}`;
        break;
      case 'DisplayClear':
        className += ' display';
        icon = '🔲';
        text = 'Display Clear';
        break;
      case 'Led':
        icon = '💡';
        text = `LED: ${event.pattern}`;
        break;
      case 'PersistSettings':
        icon = '💾';
        text = 'Settings Saved';
        break;
      case 'SyncTime':
        icon = '🕐';
        text = 'Time Sync';
        break;
      case 'RestartDevice':
        icon = '🔄';
        text = 'Device Restart';
        break;
      default:
        text = event.type;
    }

    const countSuffix = count > 1 ? ` <span class="count">×${count}</span>` : '';
    return `<div class="${className}"><span class="icon">${icon}</span><span>${text}${countSuffix}</span></div>`;
  }).reverse().join('');

  traceView.innerHTML = html;
}

// Animation loop
function animate(time: number = 0) {
  requestAnimationFrame(animate);

  const delta = time - lastTime;
  lastTime = time;

  if (simulator && running && delta > 0) {
    // Apply the speed multiplier only while the winding routine is active.
    // Otherwise, running at e.g. 50x would keep fast-forwarding "wall clock time" even after
    // the routine is stopped/finished, which is surprising.
    const timeScale = lastSimState?.routine_running ? speed : 1;

    // Step simulation (ms) - must be integer for WASM
    const simDelta = Math.floor(Math.min(delta * timeScale, 1000)); // Cap to prevent huge jumps
    if (simDelta > 0) {
      simulator.step(simDelta);
    }

    // Update state
    const state = simulator.getState() as unknown as SimState;
    lastSimState = state;

    // Update 3D rotation - rotate the whole group around Z axis (facing us)
    const targetAngle = (state.motor_angle * Math.PI) / 180;
    rotatingParts.rotation.z = targetAngle;

    // Make indicator glow brighter when motor is running
    if (motorIndicator) {
      const mat = motorIndicator.material as THREE.MeshStandardMaterial;
      if (state.motor_running) {
        mat.emissiveIntensity = 0.8 + Math.sin(time * 0.01) * 0.2; // Pulsing glow
      } else {
        mat.emissiveIntensity = 0.1;
      }
    }

    // Update at ~30fps for UI
    if (Math.floor(time / 33) !== Math.floor((time - delta) / 33)) {
      updateUI();
    }
  } else if (simulator && !running) {
    // Paused - still update UI occasionally
    if (Math.floor(time / 100) !== Math.floor((time - delta) / 100)) {
      updateUI();
    }
  }

  renderer.render(scene, camera);
}

// Start the app
init();
