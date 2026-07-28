pub mod auth;
pub mod config;
pub mod forwarding;
pub mod health;
pub mod messages;
pub mod modem;
pub mod service;

use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use axum::extract::State;
use axum::http::StatusCode;
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, Router};
use axum::Json;
use serde::Serialize;

use crate::config::AppConfig;
use crate::events::EventBus;
use crate::messaging::Messaging;
use crate::persistence::Store;

#[derive(Clone)]
pub struct ApiState {
    pub config: Arc<AppConfig>,
    pub config_path: PathBuf,
    pub config_save_lock: Arc<tokio::sync::Mutex<()>>,
    pub store: Store,
    pub events: EventBus,
    pub delivery_wakeup: crate::delivery::DeliveryWakeup,
    pub started_at: Instant,
    pub sessions: auth::SessionStore,
    pub modem: crate::modem::ModemService,
    pub sms_sender: Arc<dyn crate::dbus::SmsSender>,
    pub service_control: service::ServiceControl,
}

impl ApiState {
    pub fn messaging(&self) -> Messaging {
        Messaging::new(
            self.store.clone(),
            self.events.clone(),
            self.delivery_wakeup.clone(),
            self.sms_sender.clone(),
        )
        .with_verified_modem(self.modem.clone())
    }
}

#[cfg(test)]
pub(crate) fn test_sms_sender() -> Arc<dyn crate::dbus::SmsSender> {
    Arc::new(TestSmsSender)
}

#[cfg(test)]
struct TestSmsSender;

#[cfg(test)]
impl crate::dbus::SmsSender for TestSmsSender {
    fn prepare<'a>(
        &'a self,
        _modem_path: &'a str,
        _tel_number: &'a str,
        _sms_text: &'a str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = anyhow::Result<crate::dbus::PreparedSms>> + Send + 'a>,
    > {
        Box::pin(async {
            Ok(crate::dbus::PreparedSms {
                modem_sms_path: "/org/freedesktop/ModemManager1/SMS/test".to_string(),
            })
        })
    }

    fn send_prepared<'a>(
        &'a self,
        _modem_sms_path: &'a str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = crate::dbus::SendAttemptOutcome> + Send + 'a>,
    > {
        Box::pin(async { crate::dbus::SendAttemptOutcome::Accepted })
    }

    fn sms_state<'a>(
        &'a self,
        _modem_sms_path: &'a str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = anyhow::Result<crate::dbus::ModemSmsState>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async { Ok(crate::dbus::ModemSmsState::Sent) })
    }
}

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    error: ErrorDetail<'a>,
}

#[derive(Serialize)]
struct ErrorDetail<'a> {
    code: &'a str,
    message: &'a str,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", message)
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ErrorBody {
            error: ErrorDetail {
                code: self.code,
                message: &self.message,
            },
        };
        (self.status, Json(body)).into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

#[derive(Serialize)]
struct MonitoringPreference {
    enabled: bool,
}

async fn monitoring_preference(State(state): State<ApiState>) -> Json<MonitoringPreference> {
    Json(MonitoringPreference {
        enabled: state.config.monitoring.enabled,
    })
}

pub fn router(state: ApiState) -> Router {
    let sessions = state.sessions.clone();
    let auth_routes = auth::routes();

    let protected = Router::new()
        .merge(messages::routes())
        .merge(config::routes())
        .merge(service::routes())
        .merge(modem::routes())
        .merge(forwarding::routes())
        .layer(middleware::from_fn(
            move |req: axum::extract::Request, next: middleware::Next| {
                let sessions = sessions.clone();
                async move {
                    let token = auth::session_token(req.headers());
                    match sessions.is_valid(&token).await {
                        Ok(true) => {}
                        Ok(false) => {
                            return ApiError::unauthorized("authentication required")
                                .into_response();
                        }
                        Err(error) => {
                            return auth::session_storage_error(error).into_response();
                        }
                    }
                    next.run(req).await
                }
            },
        ));

    Router::new()
        .route("/api/monitoring", get(monitoring_preference))
        .merge(health::routes())
        .merge(auth_routes)
        .merge(protected)
        .with_state(state)
        .fallback(crate::assets::serve)
}

pub async fn serve(state: ApiState) -> anyhow::Result<()> {
    let bind = state.config.api.bind.as_str();
    let port = state.config.api.port;
    let addr = primary_listener_address(bind, port, state.config.api.enable_ipv6);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    let app = router(state.clone());
    log::info!("web api listening on {}", addr);

    if let Some(ipv6_bind) = ipv6_companion_address(bind, state.config.api.enable_ipv6)? {
        let socket = tokio::net::TcpSocket::new_v6()?;
        set_ipv6_only(&socket)?;
        socket.set_reuseaddr(true)?;
        let ipv6_addr = SocketAddr::new(IpAddr::V6(ipv6_bind), port);
        socket.bind(ipv6_addr)?;
        let ipv6_listener = socket.listen(1024)?;
        log::info!("web api listening on [{}]:{}", ipv6_bind, port);
        tokio::try_join!(
            serve_listener(listener, app.clone()),
            serve_listener(ipv6_listener, app),
        )?;
    } else {
        serve_listener(listener, app).await?;
    }
    Ok(())
}

async fn serve_listener(listener: tokio::net::TcpListener, app: Router) -> anyhow::Result<()> {
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

fn listener_address(bind: &str, port: u16) -> String {
    if is_ipv6_bind(bind) {
        format!("[{}]:{}", bind.trim_matches(['[', ']']), port)
    } else {
        format!("{bind}:{port}")
    }
}

fn primary_listener_address(bind: &str, port: u16, enable_ipv6: bool) -> String {
    if enable_ipv6 && bind.eq_ignore_ascii_case("localhost") {
        listener_address("127.0.0.1", port)
    } else {
        listener_address(bind, port)
    }
}

fn is_ipv6_bind(bind: &str) -> bool {
    bind.trim_matches(['[', ']'])
        .parse::<IpAddr>()
        .is_ok_and(|address| address.is_ipv6())
}

fn ipv6_companion_address(bind: &str, enabled: bool) -> anyhow::Result<Option<Ipv6Addr>> {
    if !enabled || is_ipv6_bind(bind) {
        return Ok(None);
    }
    let normalized = bind.trim_matches(['[', ']']);
    if normalized.eq_ignore_ascii_case("localhost") {
        return Ok(Some(Ipv6Addr::LOCALHOST));
    }
    match normalized.parse::<IpAddr>() {
        Ok(IpAddr::V4(address)) if address.is_unspecified() => Ok(Some(Ipv6Addr::UNSPECIFIED)),
        Ok(IpAddr::V4(address)) if address.is_loopback() => Ok(Some(Ipv6Addr::LOCALHOST)),
        Ok(IpAddr::V4(_)) => anyhow::bail!(
            "api.enable_ipv6 cannot infer a safe IPv6 companion for the specific IPv4 bind {bind}"
        ),
        Ok(IpAddr::V6(_)) => Ok(None),
        Err(_) => anyhow::bail!(
            "api.enable_ipv6 requires api.bind to be an IP literal or localhost, got {bind}"
        ),
    }
}

#[cfg(unix)]
fn set_ipv6_only(socket: &tokio::net::TcpSocket) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;

    let enabled: libc::c_int = 1;
    // SAFETY: the socket owns a valid file descriptor, and `enabled` is passed
    // with its exact byte size for the IPV6_V6ONLY integer socket option.
    let result = unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            libc::IPPROTO_IPV6,
            libc::IPV6_V6ONLY,
            (&enabled as *const libc::c_int).cast(),
            std::mem::size_of_val(&enabled) as libc::socklen_t,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(unix))]
fn set_ipv6_only(_socket: &tokio::net::TcpSocket) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "IPv6-only companion listeners are not supported on this platform",
    ))
}

impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        ApiError::internal(err.to_string())
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(err: rusqlite::Error) -> Self {
        ApiError::internal(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::auth::SessionStore;
    use super::config::{check_config_payload, CheckConfigPayload};
    use super::{ipv6_companion_address, listener_address, primary_listener_address};
    use crate::config::AppConfig;

    #[test]
    fn listener_address_brackets_ipv6_literals() {
        assert_eq!(listener_address("::", 8080), "[::]:8080");
        assert_eq!(listener_address("[::1]", 8080), "[::1]:8080");
        assert_eq!(listener_address("0.0.0.0", 8080), "0.0.0.0:8080");
    }

    #[test]
    fn localhost_with_ipv6_enabled_uses_an_ipv4_primary_listener() {
        assert_eq!(
            primary_listener_address("localhost", 8080, true),
            "127.0.0.1:8080"
        );
        assert_eq!(
            primary_listener_address("LOCALHOST", 8080, true),
            "127.0.0.1:8080"
        );
        assert_eq!(
            primary_listener_address("localhost", 8080, false),
            "localhost:8080"
        );
        assert_eq!(
            primary_listener_address("0.0.0.0", 8080, true),
            "0.0.0.0:8080"
        );
    }

    #[test]
    fn ipv6_companion_preserves_the_ipv4_exposure_scope() {
        assert_eq!(
            ipv6_companion_address("0.0.0.0", true).unwrap(),
            Some(std::net::Ipv6Addr::UNSPECIFIED)
        );
        assert_eq!(
            ipv6_companion_address("127.0.0.1", true).unwrap(),
            Some(std::net::Ipv6Addr::LOCALHOST)
        );
        assert_eq!(
            ipv6_companion_address("localhost", true).unwrap(),
            Some(std::net::Ipv6Addr::LOCALHOST)
        );
        assert_eq!(ipv6_companion_address("::1", true).unwrap(), None);
        assert_eq!(ipv6_companion_address("0.0.0.0", false).unwrap(), None);
        assert!(ipv6_companion_address("192.0.2.10", true).is_err());
        assert!(ipv6_companion_address("api.internal", true).is_err());
    }

    #[tokio::test]
    async fn login_cookie_uses_p2_session_contract() {
        let sessions = SessionStore::default();
        let cookie = sessions.login_cookie(false).await.unwrap();

        assert!(cookie.starts_with("sms-relayed-session="));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Lax"));
        assert!(cookie.contains("Path=/"));
        assert!(cookie.contains("Max-Age=604800"));
        assert!(!cookie.contains("Secure"));
    }

    #[tokio::test]
    async fn login_cookie_is_secure_when_request_is_https() {
        let sessions = SessionStore::default();
        let cookie = sessions.login_cookie(true).await.unwrap();
        assert!(cookie.contains("Secure"));
    }

    #[tokio::test]
    async fn session_tokens_expire_after_seven_days() {
        let sessions = SessionStore::default();
        let token = sessions.create_session().await.unwrap();
        assert!(sessions.is_valid(&token).await.unwrap());

        sessions.expire_for_test(&token).await.unwrap();
        assert!(!sessions.is_valid(&token).await.unwrap());
    }

    #[test]
    fn config_check_accepts_json_or_toml_and_rejects_bad_config() {
        let mut cfg = AppConfig::default();
        cfg.api.enabled = true;
        cfg.api.password = "secret".to_string();

        assert!(check_config_payload(CheckConfigPayload::Json(Box::new(cfg.clone()))).is_ok());

        let toml = toml::to_string_pretty(&cfg).unwrap();
        assert!(check_config_payload(CheckConfigPayload::Toml { toml }).is_ok());

        cfg.api.password.clear();
        let err = check_config_payload(CheckConfigPayload::Json(Box::new(cfg)))
            .unwrap_err()
            .to_string();
        assert!(err.contains("api.password"));
    }
}

#[cfg(test)]
mod route_tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::time::Duration;

    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use axum::http::{Method, Request, StatusCode};
    use tower::ServiceExt;

    use super::auth::SessionStore;
    use super::*;

    #[derive(Clone, Default)]
    struct ApiTestRunner;

    impl crate::modem::MmcliRunner for ApiTestRunner {
        fn run<'a>(
            &'a self,
            args: &'a [&'a str],
            _timeout: Duration,
        ) -> Pin<
            Box<
                dyn Future<Output = Result<crate::modem::MmcliOutput, crate::modem::ModemError>>
                    + Send
                    + 'a,
            >,
        > {
            Box::pin(async move {
                if args == ["--version"] {
                    return Ok(crate::modem::MmcliOutput {
                        stdout: "mmcli 1.22.0\n".to_string(),
                        stderr: String::new(),
                        status_success: true,
                    });
                }
                Ok(crate::modem::MmcliOutput {
                    stdout: include_str!("../../tests/fixtures/mmcli/healthy.json").to_string(),
                    stderr: String::new(),
                    status_success: true,
                })
            })
        }
    }

    #[derive(Clone, Default)]
    struct RecordingServiceRestarter {
        completed: Option<tokio::sync::mpsc::UnboundedSender<()>>,
    }

    impl RecordingServiceRestarter {
        fn with_completion_signal() -> (Self, tokio::sync::mpsc::UnboundedReceiver<()>) {
            let (completed, receiver) = tokio::sync::mpsc::unbounded_channel();
            (
                Self {
                    completed: Some(completed),
                },
                receiver,
            )
        }
    }

    impl service::ServiceRestarter for RecordingServiceRestarter {
        fn restart(&self) {
            if let Some(completed) = &self.completed {
                let _ = completed.send(());
            }
        }
    }

    async fn expect_restart_completed(restarts: &mut tokio::sync::mpsc::UnboundedReceiver<()>) {
        tokio::time::timeout(Duration::from_secs(2), restarts.recv())
            .await
            .expect("restart should complete")
            .expect("restart completion channel should remain open");
    }

    async fn expect_restart_idle(service_control: &service::ServiceControl) {
        tokio::time::timeout(Duration::from_secs(1), async {
            while service_control.restart_pending() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("restart scheduling should become idle after completion");
    }

    fn test_state() -> ApiState {
        let mut cfg = AppConfig::default();
        cfg.api.enabled = true;
        cfg.api.password = "secret".to_string();
        ApiState {
            config: std::sync::Arc::new(cfg),
            config_path: std::path::PathBuf::from("/tmp/sms-relayed-test.toml"),
            config_save_lock: Arc::new(tokio::sync::Mutex::new(())),
            store: crate::persistence::Store::open_in_memory().unwrap(),
            events: crate::events::EventBus::new(),
            delivery_wakeup: crate::delivery::DeliveryWakeup::new(),
            started_at: std::time::Instant::now(),
            sessions: SessionStore::default(),
            modem: crate::modem::ModemService::new_with_runner(ApiTestRunner),
            sms_sender: test_sms_sender(),
            service_control: service::ServiceControl::default(),
        }
    }

    fn write_config_file(config: &AppConfig, prefix: &str) -> (std::path::PathBuf, String) {
        let path = std::env::temp_dir().join(format!(
            "sms-relayed-{prefix}-{}.toml",
            uuid::Uuid::new_v4()
        ));
        let content = config.canonical_toml().unwrap();
        std::fs::write(&path, &content).unwrap();
        let revision = crate::config::config_revision(&content);
        (path, revision)
    }

    fn config_temporary_files(path: &std::path::Path) -> Vec<std::path::PathBuf> {
        let parent = path.parent().unwrap();
        let prefix = format!(".{}.", path.file_name().unwrap().to_string_lossy());
        std::fs::read_dir(parent)
            .unwrap()
            .filter_map(|entry| {
                let path = entry.unwrap().path();
                let name = path.file_name()?.to_string_lossy();
                (name.starts_with(&prefix) && name.ends_with(".tmp")).then_some(path)
            })
            .collect()
    }

    fn login_request(password: &str, peer: std::net::SocketAddr) -> Request<Body> {
        let mut request = Request::builder()
            .method(Method::POST)
            .uri("/api/auth/login")
            .header("content-type", "application/json")
            .body(Body::from(format!(r#"{{"password":"{password}"}}"#)))
            .unwrap();
        request.extensions_mut().insert(ConnectInfo(peer));
        request
    }

    fn proxied_login_request(
        password: &str,
        peer: std::net::SocketAddr,
        forwarded_for: &str,
    ) -> Request<Body> {
        let mut request = login_request(password, peer);
        request
            .headers_mut()
            .insert("x-forwarded-for", forwarded_for.parse().unwrap());
        request
    }

    #[tokio::test]
    async fn login_rate_limits_repeated_failures_per_client() {
        let app = router(test_state());
        let first_client = "192.0.2.10:1234".parse().unwrap();
        let other_client = "192.0.2.11:1234".parse().unwrap();

        for _ in 0..5 {
            let response = app
                .clone()
                .oneshot(login_request("wrong", first_client))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }

        let response = app
            .clone()
            .oneshot(login_request("wrong", first_client))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);

        let response = app
            .oneshot(login_request("secret", other_client))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn login_rate_limit_distinguishes_clients_behind_a_trusted_proxy() {
        let mut state = test_state();
        Arc::make_mut(&mut state.config)
            .api
            .trusted_proxies
            .push("192.0.2.1".parse().unwrap());
        let app = router(state);
        let proxy = "192.0.2.1:1234".parse().unwrap();

        for _ in 0..5 {
            let response = app
                .clone()
                .oneshot(proxied_login_request("wrong", proxy, "198.51.100.10"))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }

        let response = app
            .clone()
            .oneshot(proxied_login_request("wrong", proxy, "198.51.100.10"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);

        let response = app
            .oneshot(proxied_login_request("secret", proxy, "198.51.100.11"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn login_rate_limit_ignores_forwarded_for_from_an_untrusted_peer() {
        let app = router(test_state());
        let peer = "192.0.2.20:1234".parse().unwrap();

        for suffix in 1..=5 {
            let response = app
                .clone()
                .oneshot(proxied_login_request(
                    "wrong",
                    peer,
                    &format!("198.51.100.{suffix}"),
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }

        let response = app
            .oneshot(proxied_login_request("secret", peer, "198.51.100.6"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn trusted_proxy_uses_the_nearest_untrusted_forwarded_address() {
        let mut state = test_state();
        Arc::make_mut(&mut state.config)
            .api
            .trusted_proxies
            .push("192.0.2.1".parse().unwrap());
        let app = router(state);
        let proxy = "192.0.2.1:1234".parse().unwrap();

        for suffix in 1..=5 {
            let response = app
                .clone()
                .oneshot(proxied_login_request(
                    "wrong",
                    proxy,
                    &format!("203.0.113.{suffix}, 198.51.100.10"),
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }

        let response = app
            .oneshot(proxied_login_request(
                "secret",
                proxy,
                "203.0.113.6, 198.51.100.10",
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn trusted_ipv4_proxy_matches_an_ipv4_mapped_ipv6_peer() {
        let mut state = test_state();
        Arc::make_mut(&mut state.config)
            .api
            .trusted_proxies
            .push("192.0.2.1".parse().unwrap());
        let app = router(state);
        let proxy = "[::ffff:192.0.2.1]:1234".parse().unwrap();

        for _ in 0..5 {
            let response = app
                .clone()
                .oneshot(proxied_login_request("wrong", proxy, "198.51.100.10"))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }

        let response = app
            .oneshot(proxied_login_request("secret", proxy, "198.51.100.11"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn monitoring_preference_is_public_and_disabled_by_default() {
        let response = router(test_state())
            .oneshot(
                Request::builder()
                    .uri("/api/monitoring")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            serde_json::json!({"enabled": false})
        );
    }

    #[tokio::test]
    async fn monitoring_preference_reports_enabled_config() {
        let mut state = test_state();
        Arc::make_mut(&mut state.config).monitoring.enabled = true;
        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/api/monitoring")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            serde_json::json!({"enabled": true})
        );
    }

    #[tokio::test]
    async fn service_control_coalesces_pending_restarts_and_accepts_another_after_completion() {
        let mut state = test_state();
        let (restarter, mut restarts) = RecordingServiceRestarter::with_completion_signal();
        state.service_control = service::ServiceControl::new(restarter);

        assert!(service::schedule_restart(&state));
        assert!(state.service_control.restart_pending());
        assert!(!service::schedule_restart(&state));
        assert!(state.service_control.restart_pending());

        expect_restart_completed(&mut restarts).await;
        expect_restart_idle(&state.service_control).await;
        assert!(!state.service_control.restart_pending());

        assert!(service::schedule_restart(&state));
        expect_restart_completed(&mut restarts).await;
        expect_restart_idle(&state.service_control).await;
    }

    #[tokio::test]
    async fn changing_the_api_password_schedules_restart_and_invalidates_sessions() {
        let mut state = test_state();
        let (config_path, base_revision) = write_config_file(&state.config, "password-change");
        state.config_path = config_path.clone();
        let (restarter, mut restarts) = RecordingServiceRestarter::with_completion_signal();
        state.service_control = service::ServiceControl::new(restarter);
        let token = state.sessions.create_session().await.unwrap();
        let sessions = state.sessions.clone();
        let mut updated_config = (*state.config).clone();
        updated_config.api.password = "new-password".to_string();
        let candidate_revision =
            crate::config::config_revision(&updated_config.canonical_toml().unwrap());
        let app = router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::PUT)
                    .uri("/api/config?restart_after_save=true")
                    .header("cookie", format!("sms-relayed-session={token}"))
                    .header("content-type", "application/json")
                    .header("if-match", base_revision.clone())
                    .header("x-config-candidate-revision", candidate_revision)
                    .body(Body::from(serde_json::to_vec(&updated_config).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body["restart_scheduled"], true);
        assert_eq!(body["session_invalidated"], true);
        assert!(!sessions.is_valid(&token).await.unwrap());
        expect_restart_completed(&mut restarts).await;

        let _ = std::fs::remove_file(config_path);
    }

    #[tokio::test]
    async fn password_change_succeeds_after_rename_when_parent_sync_fails() {
        let mut state = test_state();
        let (config_path, base_revision) =
            write_config_file(&state.config, "password-parent-sync-failure");
        state.config_path = config_path.clone();
        let (restarter, mut restarts) = RecordingServiceRestarter::with_completion_signal();
        state.service_control = service::ServiceControl::new(restarter);
        let token = state.sessions.create_session().await.unwrap();
        let sessions = state.sessions.clone();
        let mut updated_config = (*state.config).clone();
        updated_config.api.password = "new-password".to_string();
        let candidate_toml = updated_config.canonical_toml().unwrap();
        let candidate_revision = crate::config::config_revision(&candidate_toml);
        crate::config::fail_next_config_parent_sync_for(&config_path);
        let app = router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::PUT)
                    .uri("/api/config?restart_after_save=true")
                    .header("cookie", format!("sms-relayed-session={token}"))
                    .header("content-type", "application/json")
                    .header("if-match", base_revision)
                    .header("x-config-candidate-revision", candidate_revision)
                    .body(Body::from(serde_json::to_vec(&updated_config).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(!sessions.is_valid(&token).await.unwrap());
        assert_eq!(
            std::fs::read_to_string(&config_path).unwrap(),
            candidate_toml
        );
        expect_restart_completed(&mut restarts).await;

        let _ = std::fs::remove_file(config_path);
    }

    #[tokio::test]
    async fn password_change_does_not_schedule_restart_when_session_invalidation_fails() {
        let mut state = test_state();
        let (config_path, base_revision) =
            write_config_file(&state.config, "password-invalidation-failure");
        state.config_path = config_path.clone();
        let original = std::fs::read_to_string(&config_path).unwrap();
        state.service_control = service::ServiceControl::new(RecordingServiceRestarter::default());
        let service_control = state.service_control.clone();
        let token = state.sessions.create_session().await.unwrap();
        state.sessions.fail_next_invalidate_all();
        let mut updated_config = (*state.config).clone();
        updated_config.api.password = "new-password".to_string();
        let candidate_revision =
            crate::config::config_revision(&updated_config.canonical_toml().unwrap());
        let app = router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::PUT)
                    .uri("/api/config?restart_after_save=true")
                    .header("cookie", format!("sms-relayed-session={token}"))
                    .header("content-type", "application/json")
                    .header("if-match", base_revision.clone())
                    .header("x-config-candidate-revision", candidate_revision)
                    .body(Body::from(serde_json::to_vec(&updated_config).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let saved = std::fs::read_to_string(&config_path).unwrap();
        assert_eq!(saved, original);
        assert_eq!(crate::config::config_revision(&saved), base_revision);
        assert!(config_temporary_files(&config_path).is_empty());
        assert!(!service_control.restart_pending());

        let _ = std::fs::remove_file(config_path);
    }

    #[tokio::test]
    async fn password_change_prepare_failure_preserves_session_file_and_restart_state() {
        let mut state = test_state();
        let (config_path, base_revision) =
            write_config_file(&state.config, "password-prepare-failure");
        state.config_path = config_path.clone();
        let original = std::fs::read_to_string(&config_path).unwrap();
        state.service_control = service::ServiceControl::new(RecordingServiceRestarter::default());
        let service_control = state.service_control.clone();
        let token = state.sessions.create_session().await.unwrap();
        let sessions = state.sessions.clone();
        crate::config::fail_next_prepare_config_write_for(&config_path);
        let mut updated_config = (*state.config).clone();
        updated_config.api.password = "new-password".to_string();
        let candidate_revision =
            crate::config::config_revision(&updated_config.canonical_toml().unwrap());
        let app = router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::PUT)
                    .uri("/api/config?restart_after_save=true")
                    .header("cookie", format!("sms-relayed-session={token}"))
                    .header("content-type", "application/json")
                    .header("if-match", base_revision.clone())
                    .header("x-config-candidate-revision", candidate_revision)
                    .body(Body::from(serde_json::to_vec(&updated_config).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(sessions.is_valid(&token).await.unwrap());
        let saved = std::fs::read_to_string(&config_path).unwrap();
        assert_eq!(saved, original);
        assert_eq!(crate::config::config_revision(&saved), base_revision);
        assert!(config_temporary_files(&config_path).is_empty());
        assert!(!service_control.restart_pending());

        let _ = std::fs::remove_file(config_path);
    }

    #[tokio::test]
    async fn password_change_commit_failure_invalidates_sessions_and_returns_internal_error() {
        let mut state = test_state();
        let (config_path, base_revision) =
            write_config_file(&state.config, "password-commit-failure");
        state.config_path = config_path.clone();
        let original = std::fs::read_to_string(&config_path).unwrap();
        state.service_control = service::ServiceControl::new(RecordingServiceRestarter::default());
        let service_control = state.service_control.clone();
        let token = state.sessions.create_session().await.unwrap();
        let sessions = state.sessions.clone();
        crate::config::fail_next_config_commit_for(&config_path);
        let mut updated_config = (*state.config).clone();
        updated_config.api.password = "new-password".to_string();
        let candidate_revision =
            crate::config::config_revision(&updated_config.canonical_toml().unwrap());
        let app = router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::PUT)
                    .uri("/api/config?restart_after_save=true")
                    .header("cookie", format!("sms-relayed-session={token}"))
                    .header("content-type", "application/json")
                    .header("if-match", base_revision.clone())
                    .header("x-config-candidate-revision", candidate_revision)
                    .body(Body::from(serde_json::to_vec(&updated_config).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body["error"]["code"], "internal_error");
        assert!(!sessions.is_valid(&token).await.unwrap());
        let saved = std::fs::read_to_string(&config_path).unwrap();
        assert_eq!(saved, original);
        assert_eq!(crate::config::config_revision(&saved), base_revision);
        assert!(config_temporary_files(&config_path).is_empty());
        assert!(!service_control.restart_pending());

        let _ = std::fs::remove_file(config_path);
    }

    #[tokio::test]
    async fn config_get_reads_disk_and_reports_pending_restart() {
        let mut state = test_state();
        let mut disk_config = (*state.config).clone();
        disk_config.app.device_name = "disk-device".to_string();
        let (config_path, revision) = write_config_file(&disk_config, "config-get");
        state.config_path = config_path.clone();
        let token = state.sessions.create_session().await.unwrap();
        let app = router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/config")
                    .header("cookie", format!("sms-relayed-session={token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get("etag").unwrap(),
            format!("\"{revision}\"").as_str()
        );
        assert_eq!(
            response.headers().get("x-config-restart-required").unwrap(),
            "true"
        );
        assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body["app"]["device_name"], "disk-device");

        let _ = std::fs::remove_file(config_path);
    }

    #[tokio::test]
    async fn config_preview_returns_validation_and_plaintext_toml_diff_without_writing() {
        let mut state = test_state();
        let (config_path, base_revision) = write_config_file(&state.config, "config-preview");
        state.config_path = config_path.clone();
        let token = state.sessions.create_session().await.unwrap();
        let mut candidate = (*state.config).clone();
        candidate.api.password = "new-visible-password".to_string();
        candidate.app.device_name = "preview-device".to_string();
        let original = std::fs::read_to_string(&config_path).unwrap();
        let app = router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/config/preview")
                    .header("cookie", format!("sms-relayed-session={token}"))
                    .header("content-type", "application/json")
                    .header("if-match", base_revision)
                    .body(Body::from(serde_json::to_vec(&candidate).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body["check"]["passed"], true);
        assert_eq!(body["password_change_pending"], true);
        let diff = body["diff"].as_str().unwrap();
        assert!(diff.contains("secret"));
        assert!(diff.contains("new-visible-password"));
        assert!(diff.contains("preview-device"));
        assert_eq!(std::fs::read_to_string(&config_path).unwrap(), original);

        let _ = std::fs::remove_file(config_path);
    }

    #[tokio::test]
    async fn config_save_rejects_a_stale_preview_without_writing() {
        let mut state = test_state();
        let (config_path, base_revision) = write_config_file(&state.config, "config-stale");
        state.config_path = config_path.clone();
        let token = state.sessions.create_session().await.unwrap();
        let mut candidate = (*state.config).clone();
        candidate.app.device_name = "candidate-device".to_string();
        let candidate_revision =
            crate::config::config_revision(&candidate.canonical_toml().unwrap());
        let mut externally_changed = std::fs::read_to_string(&config_path).unwrap();
        externally_changed.push_str("# externally changed\n");
        std::fs::write(&config_path, &externally_changed).unwrap();
        let app = router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::PUT)
                    .uri("/api/config")
                    .header("cookie", format!("sms-relayed-session={token}"))
                    .header("content-type", "application/json")
                    .header("if-match", base_revision)
                    .header("x-config-candidate-revision", candidate_revision)
                    .body(Body::from(serde_json::to_vec(&candidate).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::PRECONDITION_FAILED);
        assert_eq!(
            std::fs::read_to_string(&config_path).unwrap(),
            externally_changed
        );

        let _ = std::fs::remove_file(config_path);
    }

    #[tokio::test]
    async fn concurrent_config_saves_with_the_same_revision_allow_only_one_write() {
        let mut state = test_state();
        let (config_path, base_revision) = write_config_file(&state.config, "config-concurrent");
        state.config_path = config_path.clone();
        let token = state.sessions.create_session().await.unwrap();
        let mut first_candidate = (*state.config).clone();
        first_candidate.app.device_name = "first-candidate".to_string();
        let first_toml = first_candidate.canonical_toml().unwrap();
        let first_revision = crate::config::config_revision(&first_toml);
        let mut second_candidate = (*state.config).clone();
        second_candidate.app.device_name = "second-candidate".to_string();
        let second_toml = second_candidate.canonical_toml().unwrap();
        let second_revision = crate::config::config_revision(&second_toml);

        let save_guard = state.config_save_lock.clone().lock_owned().await;
        let app = router(state);
        let first_request = Request::builder()
            .method(Method::PUT)
            .uri("/api/config")
            .header("cookie", format!("sms-relayed-session={token}"))
            .header("content-type", "application/json")
            .header("if-match", base_revision.clone())
            .header("x-config-candidate-revision", first_revision)
            .body(Body::from(serde_json::to_vec(&first_candidate).unwrap()))
            .unwrap();
        let second_request = Request::builder()
            .method(Method::PUT)
            .uri("/api/config")
            .header("cookie", format!("sms-relayed-session={token}"))
            .header("content-type", "application/json")
            .header("if-match", base_revision)
            .header("x-config-candidate-revision", second_revision)
            .body(Body::from(serde_json::to_vec(&second_candidate).unwrap()))
            .unwrap();
        let mut first_save = tokio::spawn(app.clone().oneshot(first_request));
        let mut second_save = tokio::spawn(app.oneshot(second_request));

        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut first_save)
                .await
                .is_err()
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut second_save)
                .await
                .is_err()
        );
        drop(save_guard);

        let first_status = first_save.await.unwrap().unwrap().status();
        let second_status = second_save.await.unwrap().unwrap().status();
        match (first_status, second_status) {
            (StatusCode::OK, StatusCode::PRECONDITION_FAILED) => {
                assert_eq!(std::fs::read_to_string(&config_path).unwrap(), first_toml);
            }
            (StatusCode::PRECONDITION_FAILED, StatusCode::OK) => {
                assert_eq!(std::fs::read_to_string(&config_path).unwrap(), second_toml);
            }
            statuses => panic!("expected one successful save and one stale save, got {statuses:?}"),
        }

        let _ = std::fs::remove_file(config_path);
    }

    #[tokio::test]
    async fn health_route_is_public() {
        let app = router(test_state());
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn modem_status_route_requires_session() {
        let app = router(test_state());
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/modem/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn modem_status_route_exposes_own_number() {
        let state = test_state();
        let token = state.sessions.create_session().await.unwrap();
        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/modem/status")
                    .header("cookie", format!("sms-relayed-session={token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body["modem"]["own_number"], "+6581234567");
    }

    #[tokio::test]
    async fn reset_rejects_missing_confirmation() {
        let state = test_state();
        let token = state.sessions.create_session().await.unwrap();
        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/modem/reset")
                    .header("cookie", format!("sms-relayed-session={token}"))
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn forwarding_route_requires_session() {
        let app = router(test_state());
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/forwarding/attempts")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    async fn test_state_with_profiles(enabled: &[&str]) -> (ApiState, String) {
        let mut cfg = AppConfig::default();
        cfg.api.enabled = true;
        cfg.api.password = "secret".to_string();
        for profile_ref in enabled {
            cfg.forward.enabled.push(profile_ref.to_string());
            if let Some(name) = profile_ref.strip_prefix("bark.") {
                cfg.channels.bark.insert(
                    name.to_string(),
                    crate::config::BarkConfig {
                        server_url: "https://api.day.app".to_string(),
                        key: "test-key".to_string(),
                    },
                );
            }
        }
        let state = ApiState {
            config: std::sync::Arc::new(cfg),
            config_path: std::path::PathBuf::from("/tmp/sms-relayed-test.toml"),
            config_save_lock: Arc::new(tokio::sync::Mutex::new(())),
            store: crate::persistence::Store::open_in_memory().unwrap(),
            events: crate::events::EventBus::new(),
            delivery_wakeup: crate::delivery::DeliveryWakeup::new(),
            started_at: std::time::Instant::now(),
            sessions: SessionStore::default(),
            modem: crate::modem::ModemService::new_with_runner(ApiTestRunner),
            sms_sender: test_sms_sender(),
            service_control: service::ServiceControl::default(),
        };
        let token = state.sessions.create_session().await.unwrap();
        (state, token)
    }

    #[tokio::test]
    async fn forwarding_enabled_profile_with_empty_samples_returns_empty_array() {
        let (state, token) = test_state_with_profiles(&["bark.primary"]).await;
        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/forwarding/attempts")
                    .header("cookie", format!("sms-relayed-session={token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body["sample_limit"], 5);
        let profiles = body["profiles"].as_array().unwrap();
        assert_eq!(profiles.len(), 1);
        let p = &profiles[0];
        assert_eq!(p["profile_key"], "bark.primary");
        assert_eq!(p["configured"], true);
        assert_eq!(p["enabled"], true);
        assert_eq!(p["samples"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn forwarding_includes_disabled_configured_and_historical_profiles_once() {
        use crate::storage::{ForwardAttemptOutcome, NewForwardAttemptSample};

        let (mut state, token) = test_state_with_profiles(&["bark.primary", "bark.primary"]).await;
        Arc::get_mut(&mut state.config)
            .unwrap()
            .channels
            .bark
            .insert(
                "disabled".to_string(),
                crate::config::BarkConfig {
                    server_url: "https://api.day.app".to_string(),
                    key: "disabled-key".to_string(),
                },
            );
        state
            .store
            .sqlite()
            .record_forward_attempt(NewForwardAttemptSample {
                profile_key: "telegram.removed".to_string(),
                delivery_id: None,
                attempt_number: 1,
                started_at: "2026-07-12T17:00:00Z".to_string(),
                completed_at: "2026-07-12T17:00:01Z".to_string(),
                latency_ms: 100,
                dispatch_delay_ms: 0,
                outcome: ForwardAttemptOutcome::Success,
                error_code: None,
            })
            .unwrap();
        let app = router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/forwarding/attempts")
                    .header("cookie", format!("sms-relayed-session={token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        let profiles = body["profiles"].as_array().unwrap();
        assert_eq!(profiles.len(), 3);
        assert_eq!(profiles[0]["profile_key"], "bark.primary");
        assert_eq!(profiles[0]["configured"], true);
        assert_eq!(profiles[0]["enabled"], true);
        assert_eq!(profiles[1]["profile_key"], "bark.disabled");
        assert_eq!(profiles[1]["configured"], true);
        assert_eq!(profiles[1]["enabled"], false);
        assert_eq!(profiles[1]["samples"].as_array().unwrap().len(), 0);
        assert_eq!(profiles[2]["profile_key"], "telegram.removed");
        assert_eq!(profiles[2]["configured"], false);
        assert_eq!(profiles[2]["enabled"], false);
        assert_eq!(profiles[2]["samples"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn forwarding_enabled_profile_returns_latest_five_samples_and_shape() {
        use crate::storage::{ForwardAttemptOutcome, NewForwardAttemptSample};
        let (state, token) = test_state_with_profiles(&["bark.primary"]).await;
        // Insert 6 samples, newest with attempt 6
        for n in 1..=6 {
            state
                .store
                .sqlite()
                .record_forward_attempt(NewForwardAttemptSample {
                    profile_key: "bark.primary".to_string(),
                    delivery_id: None,
                    attempt_number: n,
                    started_at: format!("2026-07-12T17:00:{:02}Z", n - 1),
                    completed_at: format!("2026-07-12T17:00:{:02}Z", n),
                    latency_ms: n as i64 * 100,
                    dispatch_delay_ms: 0,
                    outcome: if n % 2 == 0 {
                        ForwardAttemptOutcome::Success
                    } else {
                        ForwardAttemptOutcome::TransientFailure
                    },
                    error_code: if n % 2 == 1 {
                        Some("http_timeout".to_string())
                    } else {
                        None
                    },
                })
                .unwrap();
        }

        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/forwarding/attempts")
                    .header("cookie", format!("sms-relayed-session={token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body["sample_limit"], 5);
        let profiles = body["profiles"].as_array().unwrap();
        assert_eq!(profiles.len(), 1);
        let p = &profiles[0];
        assert_eq!(p["profile_key"], "bark.primary");
        assert!(p["configured"].as_bool().unwrap());
        assert!(p["enabled"].as_bool().unwrap());
        let samples = p["samples"].as_array().unwrap();
        assert_eq!(samples.len(), 5, "must return at most 5 samples");
        // Newest first: attempt_number 6, 5, 4, 3, 2
        assert_eq!(samples[0]["attempt_number"], 6);
        assert_eq!(samples[4]["attempt_number"], 2);
        // Each sample has all fields
        for s in samples {
            assert!(s.get("attempt_number").is_some());
            assert!(s.get("is_retry").is_some());
            assert!(s.get("started_at").is_some());
            assert!(s.get("completed_at").is_some());
            assert!(s.get("latency_ms").is_some());
            assert!(s.get("dispatch_delay_ms").is_some());
            assert!(s.get("outcome").is_some());
            assert!(s.get("error_code").is_some());
        }
        // Check is_retry on attempt 6
        assert_eq!(samples[0]["is_retry"], true);
        assert_eq!(samples[0]["latency_ms"], 600);
        assert_eq!(samples[0]["dispatch_delay_ms"], 0);
        assert_eq!(samples[0]["outcome"], "success");
        assert!(samples[0]["error_code"].is_null());
        // Retry 5 error_code
        assert_eq!(samples[1]["is_retry"], true);
        assert_eq!(samples[1]["error_code"], "http_timeout");
    }

    #[tokio::test]
    async fn forwarding_api_does_not_expose_sensitive_fields() {
        use crate::storage::{ForwardAttemptOutcome, NewForwardAttemptSample};
        let (state, token) = test_state_with_profiles(&["bark.primary"]).await;
        // Actually insert a message with phone number and body
        let message = state
            .store
            .sqlite()
            .insert_message(crate::storage::NewMessage::modem_inbound(
                "+15551234567",
                "secret code is 1234",
                "2026-07-12T17:00:00Z",
                "/org/freedesktop/ModemManager1/SMS/1",
                "fingerprint",
            ))
            .unwrap();
        // Record an attempt for that message's delivery
        state
            .store
            .sqlite()
            .record_forward_attempt(NewForwardAttemptSample {
                profile_key: "bark.primary".to_string(),
                delivery_id: Some(message.id),
                attempt_number: 1,
                started_at: "2026-07-12T17:00:00Z".to_string(),
                completed_at: "2026-07-12T17:00:01Z".to_string(),
                latency_ms: 100,
                dispatch_delay_ms: 0,
                outcome: ForwardAttemptOutcome::PermanentFailure,
                error_code: Some("shell_exit_nonzero".to_string()),
            })
            .unwrap();

        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/forwarding/attempts")
                    .header("cookie", format!("sms-relayed-session={token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8_lossy(&body_bytes);
        assert!(
            !body_str.contains("+15551234567"),
            "must not contain phone number"
        );
        assert!(
            !body_str.contains("secret code is 1234"),
            "must not contain SMS body"
        );
        assert!(!body_str.contains("secret"), "must not contain tokens");
        assert!(
            !body_str.contains("1234"),
            "must not contain code from body"
        );
        // Standardized error code is safe
        assert!(
            body_str.contains("shell_exit_nonzero"),
            "standardized error must appear"
        );
        // Provider raw error must not appear
        assert!(!body_str.contains("provider_"));
    }
}
