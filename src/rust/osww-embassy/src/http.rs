//! HTTP API routing and static asset handling for Winderoo.
//!
//! The routing logic is built on top of `picoserve`, but the API behavior
//! remains aligned with the original C++ firmware. All request parsing is
//! kept in Rust so it stays testable on the host and usable in `no_std`
//! environments.

use alloc::{string::String, vec::Vec};

use picoserve::extract::{Query, State};
use picoserve::io::Read;
use picoserve::response::{Connection, Response, ResponseWriter, StatusCode};
use picoserve::{routing, ResponseSent, Router};

use winderoo_firmware::api::{PowerPayload, ResetResponse, StatusResponse, UpdatePayload};
use winderoo_firmware::model::UpdateRequest;

use crate::state::StatusCache;
use crate::tasks::RuntimeCommand;
use crate::wifi::{WifiCommand, WifiCommandSender, WifiCredentials};

/// Default CORS headers for the Winderoo API.
const CORS_HEADERS: [(&str, &str); 3] = [
    ("Access-Control-Allow-Origin", "*"),
    ("Access-Control-Allow-Methods", "GET,POST,OPTIONS"),
    (
        "Access-Control-Allow-Headers",
        "Content-Type, Access-Control-Allow-Headers, Authorization, X-Requested-With",
    ),
];

/// Content type for JSON responses.
const JSON_CONTENT_TYPE: &str = "application/json";
/// Content type for plain text responses.
const TEXT_CONTENT_TYPE: &str = "text/plain; charset=utf-8";

/// Shared API state for HTTP handlers.
#[derive(Debug, Clone)]
pub struct ApiState<'a, const N: usize> {
    /// Cached status snapshots produced by the controller task.
    pub status_cache: &'a StatusCache,
    /// Channel for pushing runtime commands to the controller task.
    pub runtime_sender: embassy_sync::channel::Sender<
        'a,
        embassy_sync::blocking_mutex::raw::NoopRawMutex,
        RuntimeCommand,
        N,
    >,
    /// Optional Wi-Fi provisioning command sender.
    pub wifi_sender: Option<WifiCommandSender<'a, N>>,
}

impl<'a, const N: usize> ApiState<'a, N> {
    /// Create a new API state container.
    pub fn new(
        status_cache: &'a StatusCache,
        runtime_sender: embassy_sync::channel::Sender<
            'a,
            embassy_sync::blocking_mutex::raw::NoopRawMutex,
            RuntimeCommand,
            N,
        >,
        wifi_sender: Option<WifiCommandSender<'a, N>>,
    ) -> Self {
        Self {
            status_cache,
            runtime_sender,
            wifi_sender,
        }
    }
}

/// API response payload used by the handler helpers.
#[derive(Debug, Clone, PartialEq)]
pub struct ApiResponse {
    /// HTTP status code.
    pub status: StatusCode,
    /// MIME content type for the response body.
    pub content_type: &'static str,
    /// Response body.
    pub body: String,
}

impl ApiResponse {
    /// Create a JSON response (status 200).
    pub fn json(body: String) -> Self {
        Self {
            status: StatusCode::OK,
            content_type: JSON_CONTENT_TYPE,
            body,
        }
    }

    /// Create a plain text error response.
    pub fn text(status: StatusCode, message: &str) -> Self {
        Self {
            status,
            content_type: TEXT_CONTENT_TYPE,
            body: String::from(message),
        }
    }

    /// Create a 204 No Content response.
    pub fn no_content() -> Self {
        Self {
            status: StatusCode::NO_CONTENT,
            content_type: TEXT_CONTENT_TYPE,
            body: String::new(),
        }
    }
}

/// Custom response body wrapper that lets us override the content type.
#[derive(Debug)]
struct ApiBody {
    content_type: &'static str,
    body: String,
}

impl picoserve::response::Content for ApiBody {
    fn content_type(&self) -> &'static str {
        self.content_type
    }

    fn content_length(&self) -> usize {
        self.body.len()
    }

    async fn write_content<W: picoserve::io::Write>(self, mut writer: W) -> Result<(), W::Error> {
        writer.write_all(self.body.as_bytes()).await
    }
}

impl picoserve::response::IntoResponse for ApiResponse {
    async fn write_to<R: Read, W: ResponseWriter<Error = R::Error>>(
        self,
        connection: Connection<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        if self.status == StatusCode::NO_CONTENT {
            return Response::empty(self.status)
                .with_headers(CORS_HEADERS)
                .write_to(connection, response_writer)
                .await;
        }

        let body = ApiBody {
            content_type: self.content_type,
            body: self.body,
        };
        Response::new(self.status, body)
            .with_headers(CORS_HEADERS)
            .write_to(connection, response_writer)
            .await
    }
}

/// Query parameters accepted by `/api/timer`.
#[derive(Debug, serde::Deserialize)]
struct TimerQuery {
    #[serde(rename = "timerEnabled")]
    timer_enabled: Option<String>,
}

/// Parsed JSON payload accepted by `/api/timer`.
#[derive(Debug, serde::Deserialize)]
struct TimerPayload {
    #[serde(rename = "timerEnabled")]
    timer_enabled: serde_json::Value,
}

/// Parsed JSON payload accepted by `/api/wifi`.
#[derive(Debug, serde::Deserialize)]
struct WifiPayload {
    ssid: String,
    password: String,
}

/// Build the full HTTP router (API + static assets).
pub fn build_router<'a, const N: usize>(
    state: ApiState<'a, N>,
) -> Router<impl picoserve::routing::PathRouter<ApiState<'a, N>>, ApiState<'a, N>> {
    use routing::{get, get_service, post};

    Router::new()
        .route("/api/status", get(handle_status).options(cors_preflight))
        .route("/api/timer", post(handle_timer).options(cors_preflight))
        .route("/api/power", post(handle_power).options(cors_preflight))
        .route("/api/update", post(handle_update).options(cors_preflight))
        .route("/api/reset", get(handle_reset).options(cors_preflight))
        .route("/api/wifi", post(handle_wifi).options(cors_preflight))
        .route("/", get_service(static_assets::INDEX_HTML))
        .route("/index.html", get_service(static_assets::INDEX_HTML))
        .route("/main.js", get_service(static_assets::MAIN_JS))
        .route("/polyfills.js", get_service(static_assets::POLYFILLS_JS))
        .route("/runtime.js", get_service(static_assets::RUNTIME_JS))
        .route("/styles.css", get_service(static_assets::STYLES_CSS))
        .route("/favicon.ico", get_service(static_assets::FAVICON))
        .route(
            "/3rdpartylicenses.txt",
            get_service(static_assets::THIRD_PARTY_LICENSES),
        )
        .route("/settings.json", get_service(static_assets::SETTINGS_JSON))
        .route(
            "/assets/i18n/en-US.json",
            get_service(static_assets::I18N_EN),
        )
        .route(
            "/assets/i18n/es-ES.json",
            get_service(static_assets::I18N_ES),
        )
        .route(
            "/assets/i18n/fr-FR.json",
            get_service(static_assets::I18N_FR),
        )
        .route(
            "/assets/i18n/de-DE.json",
            get_service(static_assets::I18N_DE),
        )
        .route(
            "/assets/i18n/pt-BR.json",
            get_service(static_assets::I18N_PT),
        )
        .with_state(state)
}

/// API handler for `/api/status`.
async fn handle_status<'a, const N: usize>(State(state): State<ApiState<'a, N>>) -> ApiResponse {
    let snapshot = state.status_cache.snapshot();
    match serde_json::to_string(&StatusResponse::from_snapshot(&snapshot)) {
        Ok(json) => ApiResponse::json(json),
        Err(_) => ApiResponse::text(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to serialize status",
        ),
    }
}

/// API handler for `/api/timer`.
async fn handle_timer<'a, const N: usize>(
    State(state): State<ApiState<'a, N>>,
    query: Query<TimerQuery>,
    body: Vec<u8>,
) -> ApiResponse {
    if let Some(timer_enabled) = &query.timer_enabled {
        let enabled = parse_bool_any(timer_enabled);
        let _ = state
            .runtime_sender
            .send(RuntimeCommand::ApplyTimer(enabled))
            .await;
        return ApiResponse::no_content();
    }

    if body.is_empty() {
        return ApiResponse::text(StatusCode::BAD_REQUEST, "Missing timer payload");
    }

    match serde_json::from_slice::<TimerPayload>(&body) {
        Ok(payload) => {
            let enabled = match payload.timer_enabled {
                serde_json::Value::Bool(value) => value,
                serde_json::Value::Number(num) => num.as_i64().unwrap_or(0) != 0,
                serde_json::Value::String(text) => parse_bool_any(&text),
                _ => false,
            };
            let _ = state
                .runtime_sender
                .send(RuntimeCommand::ApplyTimer(enabled))
                .await;
            ApiResponse::no_content()
        }
        Err(_) => ApiResponse::text(StatusCode::BAD_REQUEST, "Invalid timer payload"),
    }
}

/// API handler for `/api/power`.
async fn handle_power<'a, const N: usize>(
    State(state): State<ApiState<'a, N>>,
    body: Vec<u8>,
) -> ApiResponse {
    match serde_json::from_slice::<PowerPayload>(&body) {
        Ok(payload) => {
            let _ = state
                .runtime_sender
                .send(RuntimeCommand::ApplyPower(payload.winder_enabled))
                .await;
            ApiResponse::no_content()
        }
        Err(_) => ApiResponse::text(StatusCode::BAD_REQUEST, "Invalid power payload"),
    }
}

/// API handler for `/api/update`.
async fn handle_update<'a, const N: usize>(
    State(state): State<ApiState<'a, N>>,
    body: Vec<u8>,
) -> ApiResponse {
    match serde_json::from_slice::<UpdatePayload>(&body) {
        Ok(payload) => match UpdateRequest::try_from(payload) {
            Ok(update) => {
                let _ = state
                    .runtime_sender
                    .send(RuntimeCommand::ApplyUpdate(update))
                    .await;
                ApiResponse::no_content()
            }
            Err(_) => ApiResponse::text(StatusCode::BAD_REQUEST, "Invalid update payload"),
        },
        Err(_) => ApiResponse::text(StatusCode::BAD_REQUEST, "Invalid update payload"),
    }
}

/// API handler for `/api/reset`.
async fn handle_reset<'a, const N: usize>(State(state): State<ApiState<'a, N>>) -> ApiResponse {
    let _ = state.runtime_sender.send(RuntimeCommand::Reset).await;
    match serde_json::to_string(&ResetResponse::new()) {
        Ok(json) => ApiResponse::json(json),
        Err(_) => ApiResponse::text(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to serialize reset",
        ),
    }
}

/// API handler for `/api/wifi`.
async fn handle_wifi<'a, const N: usize>(
    State(state): State<ApiState<'a, N>>,
    body: Vec<u8>,
) -> ApiResponse {
    let Some(sender) = &state.wifi_sender else {
        return ApiResponse::text(
            StatusCode::SERVICE_UNAVAILABLE,
            "WiFi provisioning disabled",
        );
    };

    match serde_json::from_slice::<WifiPayload>(&body) {
        Ok(payload) => {
            let creds = WifiCredentials::new(payload.ssid, payload.password);
            sender.send(WifiCommand::SetCredentials(creds)).await;
            ApiResponse::no_content()
        }
        Err(_) => ApiResponse::text(StatusCode::BAD_REQUEST, "Invalid wifi payload"),
    }
}

/// API handler for CORS preflight requests.
async fn cors_preflight() -> ApiResponse {
    ApiResponse::text(StatusCode::OK, "Ok")
}

fn parse_bool_any(value: &str) -> bool {
    matches!(value, "1" | "true" | "True" | "TRUE")
}

mod static_assets {
    use picoserve::response::File;

    const CACHE_FOREVER: &[(&str, &str)] = &[("Cache-Control", "max-age=31536000")];
    const CACHE_NONE: &[(&str, &str)] = &[("Cache-Control", "no-cache")];
    const GZIP_CACHE_FOREVER: &[(&str, &str)] = &[
        ("Content-Encoding", "gzip"),
        ("Cache-Control", "max-age=31536000"),
    ];
    const GZIP_CACHE_NONE: &[(&str, &str)] =
        &[("Content-Encoding", "gzip"), ("Cache-Control", "no-cache")];

    const fn gzip_file(
        content_type: &'static str,
        body: &'static [u8],
        cache: &'static [(&'static str, &'static str)],
    ) -> File {
        File::with_content_type_and_headers(content_type, body, cache)
    }

    const fn plain_file(
        content_type: &'static str,
        body: &'static [u8],
        cache: &'static [(&'static str, &'static str)],
    ) -> File {
        File::with_content_type_and_headers(content_type, body, cache)
    }

    pub const INDEX_HTML: File = gzip_file(
        File::MIME_HTML,
        include_bytes!("../../../../data/index.html.gz"),
        GZIP_CACHE_NONE,
    );
    pub const MAIN_JS: File = gzip_file(
        File::MIME_JS,
        include_bytes!("../../../../data/main.js.gz"),
        GZIP_CACHE_FOREVER,
    );
    pub const POLYFILLS_JS: File = gzip_file(
        File::MIME_JS,
        include_bytes!("../../../../data/polyfills.js.gz"),
        GZIP_CACHE_FOREVER,
    );
    pub const RUNTIME_JS: File = gzip_file(
        File::MIME_JS,
        include_bytes!("../../../../data/runtime.js.gz"),
        GZIP_CACHE_FOREVER,
    );
    pub const STYLES_CSS: File = gzip_file(
        File::MIME_CSS,
        include_bytes!("../../../../data/styles.css.gz"),
        GZIP_CACHE_FOREVER,
    );
    pub const FAVICON: File = gzip_file(
        "image/x-icon",
        include_bytes!("../../../../data/favicon.ico.gz"),
        GZIP_CACHE_FOREVER,
    );
    pub const THIRD_PARTY_LICENSES: File = gzip_file(
        "text/plain; charset=utf-8",
        include_bytes!("../../../../data/3rdpartylicenses.txt.gz"),
        GZIP_CACHE_FOREVER,
    );
    pub const SETTINGS_JSON: File = plain_file(
        "application/json",
        include_bytes!("../../../../data/settings.json"),
        CACHE_NONE,
    );
    pub const I18N_EN: File = plain_file(
        "application/json; charset=utf-8",
        include_bytes!("../../../../data/assets/i18n/en-US.json"),
        CACHE_FOREVER,
    );
    pub const I18N_ES: File = plain_file(
        "application/json; charset=utf-8",
        include_bytes!("../../../../data/assets/i18n/es-ES.json"),
        CACHE_FOREVER,
    );
    pub const I18N_FR: File = plain_file(
        "application/json; charset=utf-8",
        include_bytes!("../../../../data/assets/i18n/fr-FR.json"),
        CACHE_FOREVER,
    );
    pub const I18N_DE: File = plain_file(
        "application/json; charset=utf-8",
        include_bytes!("../../../../data/assets/i18n/de-DE.json"),
        CACHE_FOREVER,
    );
    pub const I18N_PT: File = plain_file(
        "application/json; charset=utf-8",
        include_bytes!("../../../../data/assets/i18n/pt-BR.json"),
        CACHE_FOREVER,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::StatusCache;
    use crate::tasks::{RuntimeCommand, RuntimeCommandChannel};
    use winderoo_firmware::model::{
        Direction, RoutineState, RtcConfig, RuntimeState, ScreenSchedule, ScreenState, TimerConfig,
        WinderStatus,
    };
    use winderoo_firmware::time::TimeOfDay;

    fn sample_snapshot() -> winderoo_firmware::model::StatusSnapshot {
        winderoo_firmware::model::StatusSnapshot::from_state(
            &RuntimeState {
                status: WinderStatus::Stopped,
                rotations_per_day: 220,
                direction: Direction::Both,
                motor_direction: winderoo_firmware::model::MotorDirection::Clockwise,
                timer: TimerConfig {
                    enabled: false,
                    start_time: TimeOfDay::new(0, 0).unwrap(),
                },
                winder_enabled: true,
                custom_wind_duration_secs: 180,
                custom_wind_pause_secs: 15,
                rotation_duration_secs: 8,
                rtc: RtcConfig {
                    gmt_offset: 0.0,
                    dst: false,
                },
                screen: ScreenState {
                    equipped: false,
                    sleep: false,
                    schedule: ScreenSchedule {
                        enabled: false,
                        start: TimeOfDay::new(0, 0).unwrap(),
                        end: TimeOfDay::new(0, 0).unwrap(),
                    },
                },
                routine: RoutineState::idle(),
                cycle_progress: 0.0,
            },
            100,
            -42,
            "test",
        )
    }

    #[test]
    fn status_handler_serializes() {
        let cache = StatusCache::new(sample_snapshot());
        let runtime_channel: RuntimeCommandChannel<4> = RuntimeCommandChannel::new();
        let state = ApiState::new(&cache, runtime_channel.sender(), None);

        let response = futures::executor::block_on(handle_status(State(state)));
        assert_eq!(response.status, StatusCode::OK);
        assert!(response.body.contains("\"apiVersion\""));
    }

    #[test]
    fn timer_query_sends_command() {
        let cache = StatusCache::new(sample_snapshot());
        let runtime_channel: RuntimeCommandChannel<4> = RuntimeCommandChannel::new();
        let state = ApiState::new(&cache, runtime_channel.sender(), None);

        let query = Query(TimerQuery {
            timer_enabled: Some("1".into()),
        });
        let response = futures::executor::block_on(handle_timer(State(state), query, Vec::new()));
        assert_eq!(response.status, StatusCode::NO_CONTENT);

        let command = futures::executor::block_on(runtime_channel.receiver().receive());
        assert_eq!(command, RuntimeCommand::ApplyTimer(true));
    }

    #[test]
    fn wifi_disabled_returns_503() {
        let cache = StatusCache::new(sample_snapshot());
        let runtime_channel: RuntimeCommandChannel<4> = RuntimeCommandChannel::new();
        let state = ApiState::new(&cache, runtime_channel.sender(), None);

        let body = br#"{"ssid":"ssid","password":"pass"}"#.to_vec();

        let response = futures::executor::block_on(handle_wifi(State(state), body));
        assert_eq!(response.status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn power_payload_sends_command() {
        let cache = StatusCache::new(sample_snapshot());
        let runtime_channel: RuntimeCommandChannel<4> = RuntimeCommandChannel::new();
        let state = ApiState::new(&cache, runtime_channel.sender(), None);

        let body = br#"{"winderEnabled":true}"#.to_vec();
        let response = futures::executor::block_on(handle_power(State(state), body));
        assert_eq!(response.status, StatusCode::NO_CONTENT);

        let command = futures::executor::block_on(runtime_channel.receiver().receive());
        assert_eq!(command, RuntimeCommand::ApplyPower(true));
    }

    #[test]
    fn update_payload_sends_command() {
        let cache = StatusCache::new(sample_snapshot());
        let runtime_channel: RuntimeCommandChannel<4> = RuntimeCommandChannel::new();
        let state = ApiState::new(&cache, runtime_channel.sender(), None);

        let body = br#"{
            "rotationDirection":"BOTH",
            "tpd":"220",
            "action":"STOP",
            "hour":"00",
            "minutes":"00",
            "timerEnabled":"0",
            "screenSleep":false,
            "customWindDuration":"180",
            "customWindPauseDuration":"15",
            "customDurationInSecondsToCompleteOneRevolution":8,
            "rtcGmtOffset":0,
            "rtcDST":false
        }"#
        .to_vec();
        let response = futures::executor::block_on(handle_update(State(state), body));
        assert_eq!(response.status, StatusCode::NO_CONTENT);

        let command = futures::executor::block_on(runtime_channel.receiver().receive());
        assert!(matches!(command, RuntimeCommand::ApplyUpdate(_)));
    }

    #[test]
    fn timer_body_parses_boolean_values() {
        let cache = StatusCache::new(sample_snapshot());
        let runtime_channel: RuntimeCommandChannel<4> = RuntimeCommandChannel::new();
        let state = ApiState::new(&cache, runtime_channel.sender(), None);

        let body = br#"{"timerEnabled":true}"#.to_vec();

        let query = Query(TimerQuery {
            timer_enabled: None,
        });
        let response = futures::executor::block_on(handle_timer(State(state), query, body));
        assert_eq!(response.status, StatusCode::NO_CONTENT);

        let command = futures::executor::block_on(runtime_channel.receiver().receive());
        assert_eq!(command, RuntimeCommand::ApplyTimer(true));
    }
}
