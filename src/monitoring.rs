use std::borrow::Cow;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use sentry::protocol::{Event, Exception, Values};

const DEFAULT_DSN: &str =
    "https://32f3ad7ac2cf2f1d3d531d19e5acc2ba@o496942.ingest.us.sentry.io/4511733180465152";
const REPORT_INTERVAL: Duration = Duration::from_secs(5 * 60);

static LAST_REPORTS: OnceLock<Mutex<HashMap<&'static str, Instant>>> = OnceLock::new();

pub struct Guard(Option<sentry::ClientInitGuard>);

impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(guard) = self.0.take() {
            // Give the final event a brief delivery window, then avoid joining the
            // HTTP worker during process or panic exit. The OS will reclaim it.
            guard.flush(Some(Duration::from_millis(500)));
            std::mem::forget(guard);
        }
    }
}

pub fn init() -> Option<Guard> {
    let dsn = configured_dsn()?;
    let sentry_http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(1))
        .timeout(Duration::from_secs(2))
        .build()
        .expect("valid Sentry HTTP client config");
    let transport = move |options: &sentry::ClientOptions| -> Arc<dyn sentry::Transport> {
        Arc::new(sentry::transports::ReqwestHttpTransport::with_client(
            options,
            sentry_http.clone(),
        ))
    };
    let options = sentry::ClientOptions {
        dsn: Some(dsn),
        release: Some(Cow::Borrowed(env!("SMS_RELAYED_BUILD_VERSION"))),
        max_breadcrumbs: 0,
        send_default_pii: false,
        before_breadcrumb: Some(Arc::new(|_| None)),
        before_send: Some(Arc::new(|event| Some(scrub_event(event)))),
        shutdown_timeout: Duration::from_millis(500),
        transport: Some(Arc::new(transport)),
        ..Default::default()
    };

    Some(Guard(Some(sentry::init(options))))
}

/// Report a fixed operational error code without attaching the underlying error text.
/// Repeated failures are throttled so an offline or unhealthy modem cannot flood Sentry.
pub fn capture_failure(component: &'static str, code: &'static str) {
    if !should_report(code) {
        return;
    }

    let event = Event {
        level: sentry::Level::Error,
        fingerprint: Cow::Owned(vec![Cow::Borrowed(component), Cow::Borrowed(code)]),
        exception: Values::from(vec![Exception {
            ty: code.to_string(),
            value: None,
            ..Default::default()
        }]),
        tags: [
            ("component".to_string(), component.to_string()),
            ("error_code".to_string(), code.to_string()),
        ]
        .into_iter()
        .collect(),
        ..Default::default()
    };
    sentry::capture_event(event);
}

fn configured_dsn() -> Option<sentry::types::Dsn> {
    parse_configured_dsn(std::env::var_os("SMS_RELAYED_SENTRY_DSN"))
}

fn parse_configured_dsn(value: Option<OsString>) -> Option<sentry::types::Dsn> {
    match value {
        Some(value) if value.is_empty() => None,
        Some(value) => value.to_str().and_then(|dsn| dsn.parse().ok()),
        None => DEFAULT_DSN.parse().ok(),
    }
}

fn should_report(code: &'static str) -> bool {
    let now = Instant::now();
    let reports = LAST_REPORTS.get_or_init(|| Mutex::new(HashMap::new()));
    let Ok(mut reports) = reports.lock() else {
        return false;
    };

    if reports
        .get(code)
        .is_some_and(|last| now.duration_since(*last) < REPORT_INTERVAL)
    {
        return false;
    }
    reports.insert(code, now);
    true
}

fn scrub_event(mut event: Event<'static>) -> Event<'static> {
    if event.message.is_some() {
        event.message = Some("[redacted]".to_string());
    }
    event.logentry = None;
    event.logger = None;
    event.modules.clear();
    event.request = None;
    event.user = None;
    event.breadcrumbs.values.clear();
    event.extra.clear();
    event.server_name = None;
    event.transaction = None;
    event.culprit = None;
    event.contexts.clear();
    event
        .tags
        .retain(|key, _| matches!(key.as_str(), "component" | "error_code"));
    event.threads.values.clear();
    event.template = None;
    event.debug_meta = Default::default();
    if let Some(stacktrace) = &mut event.stacktrace {
        scrub_stacktrace(stacktrace);
    }
    for exception in &mut event.exception.values {
        if exception.value.is_some() {
            exception.value = Some("[redacted]".to_string());
        }
        if let Some(stacktrace) = &mut exception.stacktrace {
            scrub_stacktrace(stacktrace);
        }
        exception.raw_stacktrace = None;
        if let Some(mechanism) = &mut exception.mechanism {
            mechanism.description = None;
            mechanism.help_link = None;
            mechanism.data.clear();
        }
    }
    event
}

fn scrub_stacktrace(stacktrace: &mut sentry::protocol::Stacktrace) {
    stacktrace.registers.clear();
    for frame in &mut stacktrace.frames {
        frame.filename = frame
            .filename
            .as_deref()
            .or(frame.abs_path.as_deref())
            .and_then(safe_basename);
        frame.abs_path = None;
        frame.pre_context.clear();
        frame.context_line = None;
        frame.post_context.clear();
        frame.vars.clear();
    }
}

fn safe_basename(value: &str) -> Option<String> {
    if let Ok(url) = reqwest::Url::parse(value) {
        return Path::new(url.path())
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .map(str::to_string);
    }
    let without_query = value.split(['?', '#']).next().unwrap_or(value);
    Path::new(without_query)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use std::ffi::OsString;

    use sentry::protocol::{
        Breadcrumb, Event, Exception, Frame, Request, Stacktrace, User, Values,
    };

    use super::{parse_configured_dsn, scrub_event};

    #[test]
    fn scrub_event_removes_sensitive_payloads_and_keeps_stack_identity() {
        let mut event = Event {
            message: Some("SMS body 123456".to_string()),
            request: Some(Request::default()),
            user: Some(User {
                id: Some("+15550000000".to_string()),
                ..Default::default()
            }),
            breadcrumbs: Values::from(vec![Breadcrumb {
                message: Some("token=secret".to_string()),
                ..Default::default()
            }]),
            exception: Values::from(vec![Exception {
                ty: "DatabaseError".to_string(),
                value: Some("failed while handling +15550000000".to_string()),
                stacktrace: Some(Stacktrace {
                    frames: vec![
                        Frame {
                            filename: Some(
                                "https://router.local/assets/app.js?build=secret".to_string(),
                            ),
                            abs_path: Some("/Users/private/project/src/main.rs".to_string()),
                            context_line: Some("token = secret".to_string()),
                            ..Default::default()
                        },
                        Frame {
                            filename: Some("https://router.local".to_string()),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }),
                ..Default::default()
            }]),
            server_name: Some(Cow::Borrowed("router-bedroom")),
            ..Default::default()
        };
        event
            .extra
            .insert("config".to_string(), serde_json::json!({"token": "secret"}));

        let scrubbed = scrub_event(event);

        assert_eq!(scrubbed.message.as_deref(), Some("[redacted]"));
        assert!(scrubbed.request.is_none());
        assert!(scrubbed.user.is_none());
        assert!(scrubbed.breadcrumbs.values.is_empty());
        assert!(scrubbed.extra.is_empty());
        assert!(scrubbed.server_name.is_none());
        assert_eq!(scrubbed.exception.values[0].ty, "DatabaseError");
        assert_eq!(
            scrubbed.exception.values[0].value.as_deref(),
            Some("[redacted]")
        );
        let frame = &scrubbed.exception.values[0]
            .stacktrace
            .as_ref()
            .unwrap()
            .frames[0];
        assert_eq!(frame.filename.as_deref(), Some("app.js"));
        assert!(frame.abs_path.is_none());
        assert!(frame.context_line.is_none());
        assert!(scrubbed.exception.values[0]
            .stacktrace
            .as_ref()
            .unwrap()
            .frames[1]
            .filename
            .is_none());
    }

    #[test]
    fn explicit_empty_or_invalid_dsn_disables_monitoring() {
        assert!(parse_configured_dsn(Some(OsString::new())).is_none());
        assert!(parse_configured_dsn(Some(OsString::from("not-a-dsn"))).is_none());
        assert!(parse_configured_dsn(None).is_some());
    }
}
