#![no_std]
#![no_main]
#![recursion_limit = "256"]

extern crate alloc;

// Keep panic + exception handlers linked in.
use esp_backtrace as _;

use core::cell::RefCell;

use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::blocking_mutex::Mutex;
use embassy_time::Duration;
use embedded_storage::nor_flash::ReadNorFlash;
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::gpio::{DriveMode, Level, Output, OutputConfig};
use esp_hal::gpio::{Input, InputConfig, Pull};
#[cfg(feature = "oled")]
use esp_hal::i2c::master::{Config as I2cConfig, I2c};
use esp_hal::ledc::channel::ChannelIFace;
use esp_hal::ledc::timer::TimerIFace;
use esp_hal::ledc::{channel, timer, LSGlobalClkSource, Ledc, LowSpeed};
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::{rng::Rng, Config as HalConfig};
use esp_println::logger::init_logger;
use log::info;

use esp_storage::FlashStorage;

use winderoo_embassy::hardware::{EventDispatcher, LedPwmDriver, MotorDriver, MotorPwmDriver};
use winderoo_embassy::state::{StatusCache, SystemSignals, WifiStatus};
use winderoo_embassy::system::{load_runtime_state, NorFlashSettingsStore, SignalSystemHooks};
use winderoo_embassy::tasks::{ControllerTask, RuntimeCommandChannel};
use winderoo_embassy::time::{RtcClock, RtcTimeSource};
use winderoo_embassy::wifi::{
    ProvisioningConfig, WifiCommandChannel, WifiCommandSender, WifiManager,
};

use winderoo_firmware::controller::Controller;
use winderoo_firmware::hardware::{LedPattern, XorShift32};

use crate::storage::{FlashPartition, NorFlashWifiCredentialStore, SharedFlash};

#[cfg(feature = "home-assistant")]
mod home_assistant;
mod storage;

// If you are okay with using a nightly compiler, you can use `static_cell::make_static!`.
macro_rules! mk_static {
    ($t:ty, $val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        STATIC_CELL.uninit().write($val)
    }};
}

/// Runtime command queue depth.
const RUNTIME_QUEUE_DEPTH: usize = 8;
/// Wi-Fi command queue depth.
const WIFI_QUEUE_DEPTH: usize = 8;

/// Size of the reserved flash region for Winderoo persistence.
const STORAGE_TOTAL_BYTES: usize = 64 * 1024;
/// Size of the settings blob region within the reserved area.
const SETTINGS_BYTES: usize = 32 * 1024;
/// Size of the Wi-Fi credential blob region within the reserved area.
const WIFI_BYTES: usize = 32 * 1024;

/// HTTP server port.
const HTTP_PORT: u16 = 80;

/// OLED invert screen (matches the Arduino default).
#[cfg(feature = "oled")]
const OLED_INVERT_SCREEN: bool = false;
/// OLED rotation (180°) (matches the Arduino default).
#[cfg(feature = "oled")]
const OLED_ROTATE_SCREEN_180: bool = false;

/// Home Assistant MQTT broker address (e.g. `"192.168.1.10:1883"`).
///
/// Set at build time via `WINDEROO_HA_BROKER`. If left as the placeholder, HA is disabled.
#[cfg(feature = "home-assistant")]
const HOME_ASSISTANT_BROKER: &str = match option_env!("WINDEROO_HA_BROKER") {
    Some(value) => value,
    None => "YOUR_HOME_ASSISTANT_IP",
};

/// SNTP server to query (UTC).
const NTP_SERVER: embassy_net::IpEndpoint = embassy_net::IpEndpoint::new(
    embassy_net::IpAddress::Ipv4(embassy_net::Ipv4Address::new(129, 6, 15, 28)),
    123,
);

/// Shared epoch store for the software RTC (seconds).
static RTC_EPOCH: Mutex<CriticalSectionRawMutex, RefCell<u64>> = Mutex::new(RefCell::new(0));

/// Shared runtime command channel.
static RUNTIME_COMMANDS: RuntimeCommandChannel<RUNTIME_QUEUE_DEPTH> = RuntimeCommandChannel::new();
/// Shared Wi-Fi command channel.
static WIFI_COMMANDS: WifiCommandChannel<WIFI_QUEUE_DEPTH> = WifiCommandChannel::new();

#[cfg(feature = "oled")]
type DisplayDriver = winderoo_embassy::hardware::Ssd1306Display<I2c<'static, esp_hal::Blocking>>;
#[cfg(not(feature = "oled"))]
type DisplayDriver = winderoo_embassy::hardware::NoopDisplay;

#[cfg(feature = "pwm-motor")]
type MotorImpl = MotorPwmDriver<
    esp_hal::ledc::channel::Channel<'static, LowSpeed>,
    esp_hal::ledc::channel::Channel<'static, LowSpeed>,
>;
#[cfg(not(feature = "pwm-motor"))]
type MotorImpl = MotorDriver<Output<'static>, Output<'static>>;

#[derive(Debug, Clone, Copy)]
struct SharedRtc {
    epoch: &'static Mutex<CriticalSectionRawMutex, RefCell<u64>>,
}

impl SharedRtc {
    const fn new(epoch: &'static Mutex<CriticalSectionRawMutex, RefCell<u64>>) -> Self {
        Self { epoch }
    }
}

impl RtcClock for SharedRtc {
    fn now_epoch(&self) -> u64 {
        self.epoch.lock(|cell| *cell.borrow())
    }

    fn set_epoch(&mut self, epoch: u64) {
        self.epoch.lock(|cell| {
            *cell.borrow_mut() = epoch;
        })
    }
}

#[derive(Debug)]
struct LedGpioDriver<PIN> {
    pin: PIN,
}

impl<PIN> LedGpioDriver<PIN>
where
    PIN: embedded_hal::digital::OutputPin,
{
    fn new(pin: PIN) -> Self {
        Self { pin }
    }

    fn set_on(&mut self) {
        let _ = self.pin.set_high();
    }

    fn set_off(&mut self) {
        let _ = self.pin.set_low();
    }
}

impl<PIN> winderoo_embassy::hardware::LedControl for LedGpioDriver<PIN>
where
    PIN: embedded_hal::digital::OutputPin,
{
    fn apply_pattern<D: embedded_hal::delay::DelayNs>(
        &mut self,
        pattern: LedPattern,
        delay: &mut D,
    ) {
        match pattern {
            LedPattern::On => self.set_on(),
            LedPattern::Off => self.set_off(),
            LedPattern::Pulse => {
                // Minimal pulse indicator. (Future: PWM ramp.)
                self.set_on();
                delay.delay_ms(30);
                self.set_off();
            }
            LedPattern::SlowBlink => {
                for _ in 0..3 {
                    self.set_on();
                    delay.delay_ms(250);
                    self.set_off();
                    delay.delay_ms(250);
                }
            }
            LedPattern::FastBlink => {
                for _ in 0..10 {
                    self.set_on();
                    delay.delay_ms(60);
                    self.set_off();
                    delay.delay_ms(60);
                }
            }
        }
    }
}

#[embassy_executor::task(pool_size = 2)]
async fn net_task(
    mut runner: embassy_net::Runner<'static, esp_radio::wifi::WifiDevice<'static>>,
) -> ! {
    runner.run().await
}

#[embassy_executor::task]
async fn wifi_task(
    manager: WifiManager<
        'static,
        winderoo_embassy::esp32::EspWifiControl<'static>,
        NorFlashWifiCredentialStore<FlashPartition<'static>>,
        WIFI_QUEUE_DEPTH,
    >,
) -> ! {
    manager.run().await
}

#[embassy_executor::task]
async fn controller_task(
    task: ControllerTask<
        'static,
        XorShift32,
        MotorImpl,
        LedPwmDriver<esp_hal::ledc::channel::Channel<'static, LowSpeed>>,
        DisplayDriver,
        SignalSystemHooks<'static>,
        Delay,
        RtcTimeSource<SharedRtc>,
        RUNTIME_QUEUE_DEPTH,
    >,
) -> ! {
    task.run().await
}

#[embassy_executor::task]
async fn system_task(
    task: winderoo_embassy::system::SystemTask<
        'static,
        NorFlashSettingsStore<FlashPartition<'static>>,
        SharedRtc,
        winderoo_embassy::sntp::UdpSntpClient<'static, 128, 128>,
        winderoo_embassy::esp32::EspReset,
    >,
) -> ! {
    task.run().await
}

#[embassy_executor::task]
async fn dhcp_server_task(stack: embassy_net::Stack<'static>) -> ! {
    use embassy_time::Timer;
    use esp_hal_dhcp_server::simple_leaser::SimpleDhcpLeaser;
    use esp_hal_dhcp_server::structs::DhcpServerConfig;

    const AP_DNS: [core::net::Ipv4Addr; 1] = [core::net::Ipv4Addr::new(192, 168, 4, 1)];

    let config = DhcpServerConfig {
        ip: core::net::Ipv4Addr::new(192, 168, 4, 1),
        lease_time: Duration::from_secs(3600),
        gateways: &[],
        subnet: None,
        // Point clients at the AP's captive portal DNS.
        dns: &AP_DNS,
        use_captive_portal: true,
    };

    let mut leaser = SimpleDhcpLeaser {
        start: core::net::Ipv4Addr::new(192, 168, 4, 50),
        end: core::net::Ipv4Addr::new(192, 168, 4, 200),
        leases: Default::default(),
    };

    // Run forever. If we ever need to stop, we can call `esp_hal_dhcp_server::dhcp_close()`.
    let _ = esp_hal_dhcp_server::run_dhcp_server(stack, config, &mut leaser).await;

    // Should never return, but keep the task type as `!` anyway.
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}

/// Wildcard DNS responder for captive portal behavior on the provisioning AP.
///
/// Responds to most DNS queries with `192.168.4.1`, so arbitrary hostnames resolve to the device.
#[embassy_executor::task]
async fn dns_captive_task(stack: embassy_net::Stack<'static>) -> ! {
    use embassy_net::udp::{PacketMetadata, UdpSocket};

    const DNS_PORT: u16 = 53;
    const AP_IP: [u8; 4] = [192, 168, 4, 1];

    let mut rx_meta = [PacketMetadata::EMPTY; 2];
    let mut tx_meta = [PacketMetadata::EMPTY; 2];
    let mut sock_rx = [0u8; 512];
    let mut sock_tx = [0u8; 512];
    let mut query = [0u8; 512];
    let mut response = [0u8; 512];

    let mut socket = UdpSocket::new(
        stack,
        &mut rx_meta,
        &mut sock_rx,
        &mut tx_meta,
        &mut sock_tx,
    );
    socket.bind(DNS_PORT).expect("DNS bind failed");

    loop {
        let (n, remote) = match socket.recv_from(&mut query).await {
            Ok(v) => v,
            Err(_) => continue,
        };

        if let Some(resp_len) =
            winderoo_embassy::captive_portal::build_dns_wildcard_response(&query[..n], &mut response, AP_IP)
        {
            let _ = socket.send_to(&response[..resp_len], remote).await;
        }
    }
}

#[cfg(feature = "mdns")]
#[embassy_executor::task]
async fn mdns_task(stack: embassy_net::Stack<'static>) -> ! {
    use core::net::{Ipv4Addr, Ipv6Addr};

    use edge_mdns::host::{Host, Service, ServiceAnswers};
    use edge_mdns::io::{bind, Mdns, IPV4_DEFAULT_SOCKET};
    use edge_mdns::HostAnswersMdnsHandler;
    use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
    use embassy_sync::signal::Signal;

    use embassy_time::Timer;

    use edge_nal::UdpSplit;

    let broadcast: Signal<CriticalSectionRawMutex, ()> = Signal::new();
    let recv_buf: edge_mdns::buf::VecBufAccess<CriticalSectionRawMutex, 1024> =
        edge_mdns::buf::VecBufAccess::new();
    let send_buf: edge_mdns::buf::VecBufAccess<CriticalSectionRawMutex, 1024> =
        edge_mdns::buf::VecBufAccess::new();
    let udp_buffers: edge_nal_embassy::UdpBuffers<1, 1024, 1024, 2> =
        edge_nal_embassy::UdpBuffers::new();
    let udp = edge_nal_embassy::Udp::new(stack, &udp_buffers);

    loop {
        stack.wait_config_up().await;

        let ipv4 = stack
            .config_v4()
            .map(|config| config.address.address())
            .unwrap_or(Ipv4Addr::UNSPECIFIED);

        let mut socket =
            match bind(&udp, IPV4_DEFAULT_SOCKET, Some(Ipv4Addr::UNSPECIFIED), None).await {
                Ok(socket) => socket,
                Err(err) => {
                    log::warn!("mDNS bind failed: {:?}", err.erase());
                    Timer::after(Duration::from_secs(5)).await;
                    continue;
                }
            };

        let (recv, send) = socket.split();

        let mdns = Mdns::new(
            Some(Ipv4Addr::UNSPECIFIED),
            None,
            recv,
            send,
            &recv_buf,
            &send_buf,
            esp_hal::rng::Rng::new(),
            &broadcast,
        );

        let host = Host {
            hostname: "winderoo",
            ipv4,
            ipv6: Ipv6Addr::UNSPECIFIED,
            ttl: edge_mdns::domain::base::Ttl::from_secs(120),
        };
        let service = Service {
            name: "winderoo",
            priority: 0,
            weight: 0,
            service: "_winderoo",
            protocol: "_tcp",
            port: HTTP_PORT,
            service_subtypes: &[],
            txt_kvs: &[],
        };
        let handler = HostAnswersMdnsHandler::new(ServiceAnswers::new(&host, &service));

        if let Err(err) = mdns.run(handler).await {
            log::warn!("mDNS stopped: {:?}", err.erase());
            Timer::after(Duration::from_secs(1)).await;
        }
    }
}

#[embassy_executor::task(pool_size = 2)]
async fn http_server_task(
    name: &'static str,
    stack: embassy_net::Stack<'static>,
    status_cache: &'static StatusCache,
    runtime_sender: embassy_sync::channel::Sender<
        'static,
        embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
        winderoo_embassy::tasks::RuntimeCommand,
        RUNTIME_QUEUE_DEPTH,
    >,
    wifi_sender: WifiCommandSender<'static, WIFI_QUEUE_DEPTH>,
) -> ! {
    use picoserve::{Config, NoGracefulShutdown, Server, Timeouts};

    let api_state =
        winderoo_embassy::http::ApiState::new(status_cache, runtime_sender, Some(wifi_sender));
    let router = winderoo_embassy::http::build_router(api_state, name == "http-ap");

    let config = Config::new(Timeouts {
        start_read_request: Some(Duration::from_secs(10)),
        persistent_start_read_request: Some(Duration::from_secs(10)),
        read_request: Some(Duration::from_secs(10)),
        write: Some(Duration::from_secs(10)),
    })
    .close_connection_after_response();

    let mut http_buffer = [0u8; 2048];
    let mut tcp_rx_buffer = [0u8; 2048];
    let mut tcp_tx_buffer = [0u8; 2048];

    let shutdown: NoGracefulShutdown = Server::new(&router, &config, &mut http_buffer)
        .listen_and_serve(
            name,
            stack,
            HTTP_PORT,
            &mut tcp_rx_buffer,
            &mut tcp_tx_buffer,
        )
        .await;

    shutdown.into_never()
}

#[embassy_executor::task]
async fn external_button_task(
    mut button: Input<'static>,
    status_cache: &'static StatusCache,
    runtime_sender: embassy_sync::channel::Sender<
        'static,
        embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
        winderoo_embassy::tasks::RuntimeCommand,
        RUNTIME_QUEUE_DEPTH,
    >,
) -> ! {
    use embassy_time::Timer;
    use winderoo_embassy::tasks::RuntimeCommand;

    loop {
        // Rising edge = pressed (assuming pull-down).
        let _ = button.wait_for_rising_edge().await;
        let enabled = status_cache.snapshot().winder_enabled;
        let _ = runtime_sender
            .send(RuntimeCommand::ApplyPower(!enabled))
            .await;

        // Debounce.
        Timer::after(Duration::from_millis(250)).await;
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let config = HalConfig::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // Heap for alloc-heavy HTTP + JSON parsing.
    esp_alloc::heap_allocator!(size: 96 * 1024);

    init_logger(log::LevelFilter::Info);
    info!("winderoo-esp32 booting");

    // Start RTOS scheduler + time driver (required by esp-radio + embassy).
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    // Initialize Wi-Fi/BLE controller.
    let radio_init = &*mk_static!(
        esp_radio::Controller<'static>,
        esp_radio::init().expect("Failed to initialize Wi-Fi/BLE controller")
    );

    // RNG for network seeds + controller RNG seed.
    let mut rng = Rng::new();
    let net_seed = rng.random() as u64 | ((rng.random() as u64) << 32);
    let net_seed_ap = rng.random() as u64 | ((rng.random() as u64) << 32);
    let controller_seed = rng.random();

    // Create Wi-Fi controller + both interfaces (STA + AP).
    let (wifi_controller, interfaces) =
        esp_radio::wifi::new(radio_init, peripherals.WIFI, Default::default())
            .expect("Failed to initialize Wi-Fi controller");

    // Network stacks.
    let sta_config = embassy_net::Config::dhcpv4(Default::default());
    let (sta_stack, sta_runner) = embassy_net::new(
        interfaces.sta,
        sta_config,
        mk_static!(
            embassy_net::StackResources<8>,
            embassy_net::StackResources::<8>::new()
        ),
        net_seed,
    );

    let ap_config = embassy_net::Config::ipv4_static(embassy_net::StaticConfigV4 {
        address: embassy_net::Ipv4Cidr::new(embassy_net::Ipv4Address::new(192, 168, 4, 1), 24),
        gateway: Some(embassy_net::Ipv4Address::new(192, 168, 4, 1)),
        dns_servers: Default::default(),
    });

    let (ap_stack, ap_runner) = embassy_net::new(
        interfaces.ap,
        ap_config,
        mk_static!(
            embassy_net::StackResources<8>,
            embassy_net::StackResources::<8>::new()
        ),
        net_seed_ap,
    );

    spawner.spawn(net_task(sta_runner)).ok();
    spawner.spawn(net_task(ap_runner)).ok();
    spawner.spawn(dhcp_server_task(ap_stack)).ok();
    spawner.spawn(dns_captive_task(ap_stack)).ok();
    #[cfg(feature = "mdns")]
    spawner.spawn(mdns_task(sta_stack)).ok();

    // Shared flash.
    let mut flash = FlashStorage::new(peripherals.FLASH);
    #[cfg(multi_core)]
    {
        flash = flash.multicore_auto_park();
    }
    let shared_flash: &'static SharedFlash =
        mk_static!(SharedFlash, Mutex::new(RefCell::new(flash)));

    let flash_capacity = shared_flash.lock(|cell| ReadNorFlash::capacity(&*cell.borrow()));
    let storage_base = flash_capacity.checked_sub(STORAGE_TOTAL_BYTES).unwrap_or(0) as u32;

    // Flash partitions for settings + Wi-Fi credentials.
    let settings_part = FlashPartition::new(shared_flash, storage_base, SETTINGS_BYTES as u32);
    let wifi_part = FlashPartition::new(
        shared_flash,
        storage_base + SETTINGS_BYTES as u32,
        WIFI_BYTES as u32,
    );

    // Settings store.
    let mut settings_store = NorFlashSettingsStore::new(settings_part, 0, SETTINGS_BYTES);

    // Wi-Fi credential store.
    let wifi_store = NorFlashWifiCredentialStore::new(wifi_part, 0, WIFI_BYTES);

    // Shared state blocks.
    let rtc = SharedRtc::new(&RTC_EPOCH);
    let runtime_state = load_runtime_state(&mut settings_store, cfg!(feature = "oled"));
    let rng = XorShift32::new(controller_seed);
    let controller = Controller::new(runtime_state, rng);

    let initial_snapshot = controller.status_snapshot(0, -100, "4.0.1");
    let status_cache: &'static StatusCache =
        &*mk_static!(StatusCache, StatusCache::new(initial_snapshot));
    let wifi_status: &'static WifiStatus = &*mk_static!(WifiStatus, WifiStatus::new());
    let signals: &'static SystemSignals = &*mk_static!(SystemSignals, SystemSignals::new());

    // Hardware - mirror Arduino defaults.
    //
    // LEDC PWM is used for the status LED, and optionally for MX1508-style PWM motor boards.
    let mut ledc = Ledc::new(peripherals.LEDC);
    ledc.set_global_slow_clock(LSGlobalClkSource::APBClk);

    let mut pwm_timer = ledc.timer::<LowSpeed>(timer::Number::Timer0);
    pwm_timer
        .configure(timer::config::Config {
            duty: timer::config::Duty::Duty8Bit,
            clock_source: timer::LSClockSource::APBClk,
            frequency: Rate::from_hz(5_000),
        })
        .expect("LEDC timer config failed");
    let pwm_timer: &'static mut esp_hal::ledc::timer::Timer<'static, LowSpeed> =
        mk_static!(esp_hal::ledc::timer::Timer<'static, LowSpeed>, pwm_timer);

    #[cfg(feature = "pwm-motor")]
    let motor: MotorImpl = {
        // Match Arduino's MX1508 default speed (145/255).
        const MOTOR_SPEED: u8 = 145;

        let mut pwm_a = ledc.channel(channel::Number::Channel1, peripherals.GPIO25);
        pwm_a
            .configure(channel::config::Config {
                timer: &*pwm_timer,
                duty_pct: 0,
                drive_mode: DriveMode::PushPull,
            })
            .expect("LEDC motor channel A config failed");

        let mut pwm_b = ledc.channel(channel::Number::Channel2, peripherals.GPIO26);
        pwm_b
            .configure(channel::config::Config {
                timer: &*pwm_timer,
                duty_pct: 0,
                drive_mode: DriveMode::PushPull,
            })
            .expect("LEDC motor channel B config failed");

        MotorPwmDriver::new(pwm_a, pwm_b, MOTOR_SPEED)
    };

    #[cfg(not(feature = "pwm-motor"))]
    let motor: MotorImpl = {
        let motor_a = Output::new(peripherals.GPIO25, Level::Low, OutputConfig::default());
        let motor_b = Output::new(peripherals.GPIO26, Level::Low, OutputConfig::default());
        MotorDriver::new(motor_a, motor_b)
    };

    let mut led_channel = ledc.channel(channel::Number::Channel0, peripherals.GPIO0);
    led_channel
        .configure(channel::config::Config {
            timer: &*pwm_timer,
            duty_pct: 0,
            drive_mode: DriveMode::PushPull,
        })
        .expect("LEDC channel config failed");
    let led = LedPwmDriver::new(led_channel);

    let button = {
        // Default to pull-down; if your wiring uses pull-up, change to `Pull::Up`
        // and flip the edge in `external_button_task`.
        let config = InputConfig::default().with_pull(Pull::Down);
        Input::new(peripherals.GPIO13, config)
    };

    #[cfg(feature = "oled")]
    let display: DisplayDriver = {
        let config = I2cConfig::default().with_frequency(Rate::from_khz(400));
        let i2c = I2c::new(peripherals.I2C0, config)
            .expect("I2C config error")
            .with_sda(peripherals.GPIO21)
            .with_scl(peripherals.GPIO22);
        winderoo_embassy::hardware::Ssd1306Display::new(
            i2c,
            status_cache,
            OLED_INVERT_SCREEN,
            OLED_ROTATE_SCREEN_180,
        )
    };
    #[cfg(not(feature = "oled"))]
    let display: DisplayDriver = winderoo_embassy::hardware::NoopDisplay;
    let system_hooks = SignalSystemHooks::new(signals);
    let delay = Delay::new();
    let dispatcher = EventDispatcher::new(motor, led, display, system_hooks, delay);

    // Controller + time source.
    let time_source = RtcTimeSource::new(rtc);
    let controller_loop = ControllerTask::new(
        controller,
        dispatcher,
        time_source,
        status_cache,
        wifi_status,
        RUNTIME_COMMANDS.receiver(),
        "4.0.1",
        Duration::from_millis(500),
    );

    // Runtime sender used by HTTP + Wi‑Fi provisioning callbacks.
    let runtime_sender = RUNTIME_COMMANDS.sender();

    // Wi-Fi manager.
    let wifi_control = winderoo_embassy::esp32::EspWifiControl::new(wifi_controller);
    let wifi_manager = WifiManager::new(
        wifi_control,
        wifi_store,
        wifi_status,
        WIFI_COMMANDS.receiver(),
        Some(runtime_sender),
        ProvisioningConfig::new("Winderoo Setup", ""),
        10,
    );

    // System services.
    let sntp_rx_meta: &'static mut [embassy_net::udp::PacketMetadata; 1] = mk_static!(
        [embassy_net::udp::PacketMetadata; 1],
        [embassy_net::udp::PacketMetadata::EMPTY; 1]
    );
    let sntp_tx_meta: &'static mut [embassy_net::udp::PacketMetadata; 1] = mk_static!(
        [embassy_net::udp::PacketMetadata; 1],
        [embassy_net::udp::PacketMetadata::EMPTY; 1]
    );
    let sntp_rx_buf: &'static mut [u8; 128] = mk_static!([u8; 128], [0u8; 128]);
    let sntp_tx_buf: &'static mut [u8; 128] = mk_static!([u8; 128], [0u8; 128]);

    let sntp = winderoo_embassy::sntp::UdpSntpClient::<'static, 128, 128>::new(
        sta_stack,
        NTP_SERVER,
        sntp_rx_meta,
        sntp_rx_buf,
        sntp_tx_meta,
        sntp_tx_buf,
    );
    let reset = winderoo_embassy::esp32::EspReset::default();
    let system = winderoo_embassy::system::SystemTask::new(
        settings_store,
        rtc,
        sntp,
        reset,
        status_cache,
        signals,
        1,
    );

    // HTTP servers (one per stack). They can share the same state + channels.
    let wifi_sender = WifiCommandSender::new(WIFI_COMMANDS.sender());
    spawner
        .spawn(external_button_task(button, status_cache, runtime_sender))
        .ok();
    spawner
        .spawn(http_server_task(
            "http-sta",
            sta_stack,
            status_cache,
            runtime_sender,
            wifi_sender.clone(),
        ))
        .ok();
    spawner
        .spawn(http_server_task(
            "http-ap",
            ap_stack,
            status_cache,
            runtime_sender,
            wifi_sender.clone(),
        ))
        .ok();

    #[cfg(feature = "home-assistant")]
    {
        if HOME_ASSISTANT_BROKER != "YOUR_HOME_ASSISTANT_IP" {
            use embassy_ha::{
                ButtonClass, ButtonConfig, CommandPolicy, DeviceConfig, EntityCommonConfig,
                NumberConfig, NumberMode, SelectConfig, SensorClass, SensorConfig, StateClass,
                SwitchClass, SwitchConfig, TextSensorConfig,
            };

            let resources: &'static mut embassy_ha::DeviceResources = mk_static!(
                embassy_ha::DeviceResources,
                embassy_ha::DeviceResources::default()
            );
            let device = embassy_ha::new(
                resources,
                DeviceConfig {
                    device_id: "winderoo",
                    device_name: "Winderoo",
                    manufacturer: "mwood77",
                    model: "Winderoo",
                },
            );

            let power = embassy_ha::create_switch(
                &device,
                "power",
                SwitchConfig {
                    common: EntityCommonConfig {
                        name: Some("Power"),
                        icon: Some("mdi:power"),
                        ..Default::default()
                    },
                    class: SwitchClass::Switch,
                    command_policy: CommandPolicy::default(),
                },
            );
            let timer_enabled = embassy_ha::create_switch(
                &device,
                "timerEnabled",
                SwitchConfig {
                    common: EntityCommonConfig {
                        name: Some("Timer Enabled"),
                        icon: Some("mdi:timer"),
                        ..Default::default()
                    },
                    class: SwitchClass::Switch,
                    command_policy: CommandPolicy::default(),
                },
            );
            let oled = embassy_ha::create_switch(
                &device,
                "oled",
                SwitchConfig {
                    common: EntityCommonConfig {
                        name: Some("OLED"),
                        icon: Some("mdi:overscan"),
                        ..Default::default()
                    },
                    class: SwitchClass::Switch,
                    command_policy: CommandPolicy::default(),
                },
            );

            let start = embassy_ha::create_button(
                &device,
                "startButton",
                ButtonConfig {
                    common: EntityCommonConfig {
                        name: Some("Start"),
                        icon: Some("mdi:play"),
                        ..Default::default()
                    },
                    class: ButtonClass::Generic,
                },
            );
            let stop = embassy_ha::create_button(
                &device,
                "stopButton",
                ButtonConfig {
                    common: EntityCommonConfig {
                        name: Some("Stop"),
                        icon: Some("mdi:stop"),
                        ..Default::default()
                    },
                    class: ButtonClass::Generic,
                },
            );

            let rpd = embassy_ha::create_number(
                &device,
                "rpd",
                NumberConfig {
                    common: EntityCommonConfig {
                        name: Some("Rotations Per Day"),
                        icon: Some("mdi:rotate-3d-variant"),
                        ..Default::default()
                    },
                    min: Some(100.0),
                    max: Some(960.0),
                    step: Some(10.0),
                    mode: NumberMode::Box,
                    command_policy: CommandPolicy::default(),
                    ..Default::default()
                },
            );

            let direction = embassy_ha::create_select(
                &device,
                "direction",
                SelectConfig {
                    common: EntityCommonConfig {
                        name: Some("Direction"),
                        icon: Some("mdi:arrow-left-right"),
                        ..Default::default()
                    },
                    options: &home_assistant::DIRECTION_OPTIONS,
                    command_policy: CommandPolicy::default(),
                },
            );

            let hour = embassy_ha::create_select(
                &device,
                "hour",
                SelectConfig {
                    common: EntityCommonConfig {
                        name: Some("Hour"),
                        icon: Some("mdi:timer-sand-full"),
                        ..Default::default()
                    },
                    options: &home_assistant::HOUR_OPTIONS,
                    command_policy: CommandPolicy::default(),
                },
            );
            let minutes = embassy_ha::create_select(
                &device,
                "minutes",
                SelectConfig {
                    common: EntityCommonConfig {
                        name: Some("Minutes"),
                        icon: Some("mdi:timer-sand-empty"),
                        ..Default::default()
                    },
                    options: &home_assistant::MINUTE_OPTIONS,
                    command_policy: CommandPolicy::default(),
                },
            );

            let rssi_reception = embassy_ha::create_sensor(
                &device,
                "rssiReception",
                SensorConfig {
                    common: EntityCommonConfig {
                        name: Some("WiFi Reception"),
                        icon: Some("mdi:antenna"),
                        ..Default::default()
                    },
                    class: SensorClass::SignalStrength,
                    state_class: StateClass::Measurement,
                    unit: Some(embassy_ha::constants::HA_UNIT_SIGNAL_STRENGTH_DBM),
                    ..Default::default()
                },
            );
            let activity = embassy_ha::create_text_sensor(
                &device,
                "activity",
                TextSensorConfig {
                    common: EntityCommonConfig {
                        name: Some("Status"),
                        icon: Some("mdi:information"),
                        ..Default::default()
                    },
                },
            );
            let current_epoch = embassy_ha::create_text_sensor(
                &device,
                "currentEpoch",
                TextSensorConfig {
                    common: EntityCommonConfig {
                        name: Some("RTC Epoch Time"),
                        icon: Some("mdi:clock-time-nine-outline"),
                        ..Default::default()
                    },
                },
            );

            let custom_wind_duration = embassy_ha::create_number(
                &device,
                "customWindDuration",
                NumberConfig {
                    common: EntityCommonConfig {
                        name: Some("Time to Rotate"),
                        icon: Some("mdi:play-circle-outline"),
                        ..Default::default()
                    },
                    min: Some(100.0),
                    max: Some(960.0),
                    step: Some(10.0),
                    mode: NumberMode::Box,
                    command_policy: CommandPolicy::default(),
                    ..Default::default()
                },
            );
            let custom_wind_pause = embassy_ha::create_number(
                &device,
                "customWindPauseDuration",
                NumberConfig {
                    common: EntityCommonConfig {
                        name: Some("Time to pause"),
                        icon: Some("mdi:pause-circle-outline"),
                        ..Default::default()
                    },
                    min: Some(10.0),
                    max: Some(900.0),
                    step: Some(5.0),
                    mode: NumberMode::Box,
                    command_policy: CommandPolicy::default(),
                    ..Default::default()
                },
            );
            let rotation_duration = embassy_ha::create_number(
                &device,
                "customDurationInSecondsToCompleteOneRevolution",
                NumberConfig {
                    common: EntityCommonConfig {
                        name: Some("Duration to complete a single rotation"),
                        icon: Some("mdi:arrow-u-down-right"),
                        ..Default::default()
                    },
                    min: Some(1.0),
                    max: Some(16.0),
                    step: Some(1.0),
                    mode: NumberMode::Box,
                    command_policy: CommandPolicy::default(),
                    ..Default::default()
                },
            );

            let rtc_offset = embassy_ha::create_select(
                &device,
                "rtcGmtOffset",
                SelectConfig {
                    common: EntityCommonConfig {
                        name: Some("UTC Offset"),
                        icon: Some("mdi:clock-time-eight-outline"),
                        ..Default::default()
                    },
                    options: &home_assistant::UTC_OFFSET_OPTIONS,
                    command_policy: CommandPolicy::default(),
                },
            );
            let rtc_dst = embassy_ha::create_switch(
                &device,
                "rtcDST",
                SwitchConfig {
                    common: EntityCommonConfig {
                        name: Some("DST"),
                        icon: Some("mdi:clock-time-four-outline"),
                        ..Default::default()
                    },
                    class: SwitchClass::Switch,
                    command_policy: CommandPolicy::default(),
                },
            );

            let screen_schedule_enabled = embassy_ha::create_switch(
                &device,
                "screenScheduleEnabled",
                SwitchConfig {
                    common: EntityCommonConfig {
                        name: Some("Screen Schedule Enabled"),
                        icon: Some("mdi:calendar-clock"),
                        ..Default::default()
                    },
                    class: SwitchClass::Switch,
                    command_policy: CommandPolicy::default(),
                },
            );

            let screen_schedule_start_hour = embassy_ha::create_select(
                &device,
                "screenScheduleStartHour",
                SelectConfig {
                    common: EntityCommonConfig {
                        name: Some("Screen Schedule Start Hour"),
                        icon: Some("mdi:clock-outline"),
                        ..Default::default()
                    },
                    options: &home_assistant::HOUR_OPTIONS,
                    command_policy: CommandPolicy::default(),
                },
            );
            let screen_schedule_start_minute = embassy_ha::create_select(
                &device,
                "screenScheduleStartMinute",
                SelectConfig {
                    common: EntityCommonConfig {
                        name: Some("Screen Schedule Start Minute"),
                        icon: Some("mdi:clock-outline"),
                        ..Default::default()
                    },
                    options: &home_assistant::MINUTE_OPTIONS,
                    command_policy: CommandPolicy::default(),
                },
            );

            let screen_schedule_end_hour = embassy_ha::create_select(
                &device,
                "screenScheduleEndHour",
                SelectConfig {
                    common: EntityCommonConfig {
                        name: Some("Screen Schedule End Hour"),
                        icon: Some("mdi:clock-outline"),
                        ..Default::default()
                    },
                    options: &home_assistant::HOUR_OPTIONS,
                    command_policy: CommandPolicy::default(),
                },
            );
            let screen_schedule_end_minute = embassy_ha::create_select(
                &device,
                "screenScheduleEndMinute",
                SelectConfig {
                    common: EntityCommonConfig {
                        name: Some("Screen Schedule End Minute"),
                        icon: Some("mdi:clock-outline"),
                        ..Default::default()
                    },
                    options: &home_assistant::MINUTE_OPTIONS,
                    command_policy: CommandPolicy::default(),
                },
            );

            spawner
                .spawn(home_assistant::ha_run_task(
                    sta_stack,
                    device,
                    HOME_ASSISTANT_BROKER,
                ))
                .ok();
            spawner
                .spawn(home_assistant::ha_power_task(
                    status_cache,
                    runtime_sender,
                    power,
                ))
                .ok();
            spawner
                .spawn(home_assistant::ha_timer_enabled_task(
                    status_cache,
                    runtime_sender,
                    timer_enabled,
                ))
                .ok();
            spawner
                .spawn(home_assistant::ha_oled_task(
                    status_cache,
                    runtime_sender,
                    oled,
                ))
                .ok();
            spawner
                .spawn(home_assistant::ha_start_button_task(
                    status_cache,
                    runtime_sender,
                    start,
                ))
                .ok();
            spawner
                .spawn(home_assistant::ha_stop_button_task(
                    status_cache,
                    runtime_sender,
                    stop,
                ))
                .ok();
            spawner
                .spawn(home_assistant::ha_rpd_task(
                    status_cache,
                    runtime_sender,
                    rpd,
                ))
                .ok();
            spawner
                .spawn(home_assistant::ha_direction_select_task(
                    status_cache,
                    runtime_sender,
                    direction,
                ))
                .ok();
            spawner
                .spawn(home_assistant::ha_timer_hour_task(
                    status_cache,
                    runtime_sender,
                    hour,
                ))
                .ok();
            spawner
                .spawn(home_assistant::ha_timer_minutes_task(
                    status_cache,
                    runtime_sender,
                    minutes,
                ))
                .ok();
            spawner
                .spawn(home_assistant::ha_custom_wind_duration_task(
                    status_cache,
                    runtime_sender,
                    custom_wind_duration,
                ))
                .ok();
            spawner
                .spawn(home_assistant::ha_custom_wind_pause_task(
                    status_cache,
                    runtime_sender,
                    custom_wind_pause,
                ))
                .ok();
            spawner
                .spawn(home_assistant::ha_rotation_duration_task(
                    status_cache,
                    runtime_sender,
                    rotation_duration,
                ))
                .ok();
            spawner
                .spawn(home_assistant::ha_rtc_offset_select_task(
                    status_cache,
                    runtime_sender,
                    rtc_offset,
                ))
                .ok();
            spawner
                .spawn(home_assistant::ha_rtc_dst_task(
                    status_cache,
                    runtime_sender,
                    rtc_dst,
                ))
                .ok();
            spawner
                .spawn(home_assistant::ha_screen_schedule_enabled_task(
                    status_cache,
                    runtime_sender,
                    screen_schedule_enabled,
                ))
                .ok();
            spawner
                .spawn(home_assistant::ha_screen_schedule_start_hour_task(
                    status_cache,
                    runtime_sender,
                    screen_schedule_start_hour,
                ))
                .ok();
            spawner
                .spawn(home_assistant::ha_screen_schedule_start_minute_task(
                    status_cache,
                    runtime_sender,
                    screen_schedule_start_minute,
                ))
                .ok();
            spawner
                .spawn(home_assistant::ha_screen_schedule_end_hour_task(
                    status_cache,
                    runtime_sender,
                    screen_schedule_end_hour,
                ))
                .ok();
            spawner
                .spawn(home_assistant::ha_screen_schedule_end_minute_task(
                    status_cache,
                    runtime_sender,
                    screen_schedule_end_minute,
                ))
                .ok();
            spawner
                .spawn(home_assistant::ha_rssi_reception_task(
                    status_cache,
                    rssi_reception,
                ))
                .ok();
            spawner
                .spawn(home_assistant::ha_activity_task(status_cache, activity))
                .ok();
            spawner
                .spawn(home_assistant::ha_current_epoch_task(
                    status_cache,
                    current_epoch,
                ))
                .ok();
        } else {
            info!("Home Assistant disabled (set WINDEROO_HA_BROKER to enable)");
        }
    }

    spawner.spawn(wifi_task(wifi_manager)).ok();
    spawner.spawn(controller_task(controller_loop)).ok();
    spawner.spawn(system_task(system)).ok();

    loop {
        // Best-effort initial time sync: keep requesting until the RTC looks "set".
        if wifi_status.is_connected() && rtc.now_epoch() < 60 {
            signals.request_sync();
        }
        embassy_time::Timer::after(Duration::from_secs(60)).await;
    }
}
