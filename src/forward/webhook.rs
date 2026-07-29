use std::collections::HashSet;

use anyhow::{bail, Context, Result};
use reqwest::header::{HeaderName, HeaderValue, CONTENT_TYPE};

use crate::config::{WebhookConfig, WebhookMethod};
use crate::forward::ForwardOutcome;

const MAX_URL_BYTES: usize = 8 * 1024;
const MAX_BODY_BYTES: usize = 256 * 1024;
const MAX_HEADERS: usize = 64;
const MAX_HEADER_NAME_BYTES: usize = 256;
const MAX_HEADER_VALUE_BYTES: usize = 8 * 1024;
const MAX_HEADER_BYTES: usize = 64 * 1024;
const TOKENS: [(&str, ValueKind, Encoding); 9] = [
    ("{MESSAGE}", ValueKind::Message, Encoding::Raw),
    ("{MESSAGE_URL}", ValueKind::Message, Encoding::Url),
    ("{MESSAGE_JSON}", ValueKind::Message, Encoding::Json),
    ("{SENDER}", ValueKind::Sender, Encoding::Raw),
    ("{SENDER_URL}", ValueKind::Sender, Encoding::Url),
    ("{SENDER_JSON}", ValueKind::Sender, Encoding::Json),
    ("{DATETIME}", ValueKind::Datetime, Encoding::Raw),
    ("{DATETIME_URL}", ValueKind::Datetime, Encoding::Url),
    ("{DATETIME_JSON}", ValueKind::Datetime, Encoding::Json),
];

#[derive(Clone, Copy)]
enum ValueKind {
    Message,
    Sender,
    Datetime,
}

#[derive(Clone, Copy)]
enum Encoding {
    Raw,
    Url,
    Json,
}

#[derive(Clone, Copy)]
pub struct WebhookMessage<'a> {
    pub sender: &'a str,
    pub message: &'a str,
    pub datetime: &'a str,
}

pub fn validate_profile(profile: &WebhookConfig) -> Result<()> {
    if profile.url.is_empty() {
        bail!("url is required");
    }
    if profile.url.len() > MAX_URL_BYTES {
        bail!("url template exceeds {MAX_URL_BYTES} bytes");
    }
    if profile.body.len() > MAX_BODY_BYTES {
        bail!("body template exceeds {MAX_BODY_BYTES} bytes");
    }
    validate_template(&profile.url)?;
    validate_static_url_authority(&profile.url)?;
    validate_template(&profile.body)?;

    let sample = WebhookMessage {
        sender: "+10000000000",
        message: "message",
        datetime: "2026-01-01T00:00:00Z",
    };
    let rendered_url = render_template(&profile.url, sample)?;
    let parsed = reqwest::Url::parse(&rendered_url).context("url template is not a valid URL")?;
    if !matches!(parsed.scheme(), "http" | "https") {
        bail!("url must use http or https");
    }

    if profile.method == WebhookMethod::Post {
        if profile.content_type.trim().is_empty() {
            bail!("content_type is required for POST");
        }
        if profile.content_type.len() > MAX_HEADER_VALUE_BYTES {
            bail!("content_type exceeds {MAX_HEADER_VALUE_BYTES} bytes");
        }
        HeaderValue::from_str(&profile.content_type).context("content_type is invalid")?;
    }
    if profile.headers.len() > MAX_HEADERS {
        bail!("too many webhook headers (maximum {MAX_HEADERS})");
    }
    let mut total_header_bytes = if profile.method == WebhookMethod::Post {
        CONTENT_TYPE.as_str().len() + profile.content_type.len()
    } else {
        0
    };
    let mut header_names = HashSet::new();
    for (name, value) in &profile.headers {
        if !header_names.insert(name.to_ascii_lowercase()) {
            bail!("duplicate webhook header name {name:?}");
        }
        if name.len() > MAX_HEADER_NAME_BYTES {
            bail!("webhook header name exceeds {MAX_HEADER_NAME_BYTES} bytes");
        }
        total_header_bytes += name.len() + value.len();
        if total_header_bytes > MAX_HEADER_BYTES {
            bail!("webhook headers exceed {MAX_HEADER_BYTES} bytes");
        }
        HeaderName::from_bytes(name.as_bytes())
            .with_context(|| format!("invalid webhook header name {name:?}"))?;
        HeaderValue::from_str(value)
            .with_context(|| format!("invalid value for webhook header {name:?}"))?;
        if value.len() > MAX_HEADER_VALUE_BYTES {
            bail!("webhook header {name:?} exceeds {MAX_HEADER_VALUE_BYTES} bytes");
        }
        if TOKENS.iter().any(|(token, _, _)| value.contains(token)) {
            bail!("webhook header values are static and cannot contain template variables");
        }
        if name.eq_ignore_ascii_case("content-type")
            || name.eq_ignore_ascii_case("host")
            || name.eq_ignore_ascii_case("content-length")
            || name.eq_ignore_ascii_case("transfer-encoding")
            || name.eq_ignore_ascii_case("connection")
        {
            bail!("webhook header {name:?} is managed by the HTTP client");
        }
    }
    Ok(())
}

fn validate_static_url_authority(template: &str) -> Result<()> {
    let authority_start = template.find("://").map_or(0, |index| index + 3);
    let authority_end = template[authority_start..]
        .find(['/', '?', '#'])
        .map_or(template.len(), |index| authority_start + index);
    let authority = &template[authority_start..authority_end];

    if TOKENS.iter().any(|(token, _, _)| authority.contains(token)) {
        bail!("webhook url authority must not contain template variables");
    }

    Ok(())
}

pub async fn send(
    client: &reqwest::Client,
    message: WebhookMessage<'_>,
    profile: &WebhookConfig,
) -> ForwardOutcome {
    let url = match render_template(&profile.url, message) {
        Ok(url) if url.len() <= MAX_URL_BYTES => url,
        Ok(_) => return permanent("webhook_url_too_large"),
        Err(_) => return permanent("webhook_template_invalid"),
    };
    let url = match reqwest::Url::parse(&url) {
        Ok(url) if matches!(url.scheme(), "http" | "https") => url,
        _ => return permanent("webhook_invalid_url"),
    };

    let mut request = match profile.method {
        WebhookMethod::Get => client.get(url),
        WebhookMethod::Post => {
            let body = match render_template(&profile.body, message) {
                Ok(body) if body.len() <= MAX_BODY_BYTES => body,
                Ok(_) => return permanent("webhook_body_too_large"),
                Err(_) => return permanent("webhook_template_invalid"),
            };
            client
                .post(url)
                .header(CONTENT_TYPE, profile.content_type.as_str())
                .body(body)
        }
    };
    for (name, value) in &profile.headers {
        request = request.header(name, value);
    }

    match request.send().await {
        Ok(response) => classify_status(response.status()),
        Err(error) => crate::forward::transport_failure(&error),
    }
}

fn permanent(code: &str) -> ForwardOutcome {
    ForwardOutcome::PermanentFailure(code.to_string())
}

fn classify_status(status: reqwest::StatusCode) -> ForwardOutcome {
    if status.is_success() {
        return ForwardOutcome::Success;
    }
    let code = format!("http_status_{}", status.as_u16());
    if status == reqwest::StatusCode::REQUEST_TIMEOUT
        || status == reqwest::StatusCode::TOO_EARLY
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
    {
        ForwardOutcome::TransientFailure(code)
    } else {
        ForwardOutcome::PermanentFailure(code)
    }
}

fn validate_template(template: &str) -> Result<()> {
    render_template(
        template,
        WebhookMessage {
            sender: "",
            message: "",
            datetime: "",
        },
    )
    .map(|_| ())
}

fn render_template(template: &str, message: WebhookMessage<'_>) -> Result<String> {
    let mut output = String::with_capacity(template.len());
    let mut offset = 0;
    while offset < template.len() {
        let rest = &template[offset..];
        if let Some((token, kind, encoding)) =
            TOKENS.iter().find(|(token, _, _)| rest.starts_with(token))
        {
            let value = match kind {
                ValueKind::Message => message.message,
                ValueKind::Sender => message.sender,
                ValueKind::Datetime => message.datetime,
            };
            match encoding {
                Encoding::Raw => output.push_str(value),
                Encoding::Url => output.push_str(&urlencoding::encode(value)),
                Encoding::Json => output.push_str(
                    &serde_json::to_string(value).expect("serializing a string cannot fail"),
                ),
            }
            offset += token.len();
            continue;
        }

        let ch = rest.chars().next().expect("offset is within the string");
        if ch == '{' {
            if let Some(close) = rest.find('}') {
                let candidate = &rest[1..close];
                if !candidate.is_empty()
                    && candidate.bytes().all(|byte| {
                        byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'
                    })
                {
                    bail!("unknown webhook template variable {{{candidate}}}");
                }
            } else {
                let candidate = &rest[1..];
                if !candidate.is_empty()
                    && candidate.bytes().all(|byte| {
                        byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'
                    })
                {
                    bail!("incomplete webhook template variable {{{candidate}");
                }
            }
        }
        output.push(ch);
        offset += ch.len_utf8();
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use axum::body::Bytes;
    use axum::extract::State;
    use axum::http::{HeaderMap, Method, Uri};
    use axum::routing::any;
    use axum::Router;

    use super::*;

    #[derive(Clone)]
    struct CapturedRequest {
        method: Method,
        uri: Uri,
        headers: HeaderMap,
        body: Bytes,
    }

    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Option<CapturedRequest>>>);

    async fn capture(
        State(state): State<Capture>,
        method: Method,
        uri: Uri,
        headers: HeaderMap,
        body: Bytes,
    ) -> reqwest::StatusCode {
        *state.0.lock().unwrap() = Some(CapturedRequest {
            method,
            uri,
            headers,
            body,
        });
        reqwest::StatusCode::NO_CONTENT
    }

    async fn server() -> (Capture, std::net::SocketAddr, tokio::task::JoinHandle<()>) {
        let state = Capture::default();
        let app = Router::new()
            .route("/{*path}", any(capture))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (state, address, task)
    }

    #[test]
    fn renders_raw_url_and_json_tokens_once() {
        let rendered = render_template(
            "{SENDER_URL}|{MESSAGE_JSON}|{DATETIME}|{MESSAGE}",
            WebhookMessage {
                sender: "+1 23",
                message: "quote \" and {SENDER}",
                datetime: "2026-07-28T12:00:00+08:00",
            },
        )
        .unwrap();

        assert_eq!(
            rendered,
            "%2B1%2023|\"quote \\\" and {SENDER}\"|2026-07-28T12:00:00+08:00|quote \" and {SENDER}"
        );
    }

    #[test]
    fn rejects_unknown_template_variable() {
        let error = render_template(
            "{TITLE}",
            WebhookMessage {
                sender: "",
                message: "",
                datetime: "",
            },
        )
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("unknown webhook template variable"));
    }

    #[test]
    fn rejects_incomplete_unknown_template_variable() {
        let error = render_template(
            "{TITLE",
            WebhookMessage {
                sender: "",
                message: "",
                datetime: "",
            },
        )
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("incomplete webhook template variable {TITLE"));
    }

    #[test]
    fn rejects_template_variables_in_url_authority() {
        for url in [
            "https://{MESSAGE}/notify",
            "https://user:{SENDER_JSON}@example.com/notify",
            "https://example.com:{DATETIME_URL}/notify",
        ] {
            let profile = WebhookConfig {
                url: url.to_string(),
                ..WebhookConfig::default()
            };

            let error = validate_profile(&profile).unwrap_err();
            assert!(
                error.to_string().contains("authority"),
                "unexpected error for {url}: {error}"
            );
        }
    }

    #[test]
    fn allows_template_variables_after_url_authority() {
        let profile = WebhookConfig {
            url: "https://example.com/{SENDER_URL}?message={MESSAGE_URL}".to_string(),
            ..WebhookConfig::default()
        };

        validate_profile(&profile).unwrap();
    }

    #[test]
    fn rejects_case_insensitive_duplicate_header_names() {
        let profile = WebhookConfig {
            url: "https://example.com/message".to_string(),
            headers: [
                ("X-Api-Key".to_string(), "one".to_string()),
                ("x-api-key".to_string(), "two".to_string()),
            ]
            .into_iter()
            .collect(),
            ..WebhookConfig::default()
        };

        let error = validate_profile(&profile).unwrap_err().to_string();
        assert!(error.contains("duplicate webhook header name"));
    }

    #[test]
    fn webhook_statuses_follow_retry_contract() {
        assert_eq!(
            classify_status(reqwest::StatusCode::FOUND),
            ForwardOutcome::PermanentFailure("http_status_302".to_string())
        );
        assert_eq!(
            classify_status(reqwest::StatusCode::TOO_EARLY),
            ForwardOutcome::TransientFailure("http_status_425".to_string())
        );
        assert_eq!(
            classify_status(reqwest::StatusCode::INTERNAL_SERVER_ERROR),
            ForwardOutcome::TransientFailure("http_status_500".to_string())
        );
    }

    #[tokio::test]
    async fn get_renders_url_and_sends_no_body() {
        let (capture, address, server) = server().await;
        let profile = WebhookConfig {
            method: WebhookMethod::Get,
            url: format!("http://{address}/send/{{SENDER_URL}}?message={{MESSAGE_URL}}"),
            content_type: String::new(),
            body: String::new(),
            headers: BTreeMap::new(),
        };

        let outcome = send(
            &crate::runner::build_webhook_http_client(&crate::config::HttpSection::default()),
            WebhookMessage {
                sender: "+1 23",
                message: "hello & world",
                datetime: "now",
            },
            &profile,
        )
        .await;
        server.abort();

        assert_eq!(outcome, ForwardOutcome::Success);
        let request = capture.0.lock().unwrap().clone().unwrap();
        assert_eq!(request.method, Method::GET);
        assert_eq!(request.uri.path(), "/send/%2B1%2023");
        assert_eq!(request.uri.query(), Some("message=hello%20%26%20world"));
        assert!(request.body.is_empty());
    }

    #[tokio::test]
    async fn post_renders_json_body_content_type_and_static_headers() {
        let (capture, address, server) = server().await;
        let profile = WebhookConfig {
            url: format!("http://{address}/message"),
            headers: BTreeMap::from([("X-Api-Key".to_string(), "secret".to_string())]),
            ..WebhookConfig::default()
        };

        let outcome = send(
            &crate::runner::build_webhook_http_client(&crate::config::HttpSection::default()),
            WebhookMessage {
                sender: "+123",
                message: "hello \"world\"",
                datetime: "2026-07-28T12:00:00+08:00",
            },
            &profile,
        )
        .await;
        server.abort();

        assert_eq!(outcome, ForwardOutcome::Success);
        let request = capture.0.lock().unwrap().clone().unwrap();
        assert_eq!(request.method, Method::POST);
        assert_eq!(request.headers["content-type"], "application/json");
        assert_eq!(request.headers["x-api-key"], "secret");
        let json: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(json["sender"], "+123");
        assert_eq!(json["message"], "hello \"world\"");
    }

    #[tokio::test]
    async fn redirects_are_not_followed_and_are_permanent() {
        let app = Router::new()
            .route(
                "/start",
                axum::routing::get(|| async {
                    (
                        axum::http::StatusCode::FOUND,
                        [(axum::http::header::LOCATION, "/end")],
                    )
                }),
            )
            .route(
                "/end",
                axum::routing::get(|| async { axum::http::StatusCode::NO_CONTENT }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let profile = WebhookConfig {
            method: WebhookMethod::Get,
            url: format!("http://{address}/start"),
            ..WebhookConfig::default()
        };

        let outcome = send(
            &crate::runner::build_webhook_http_client(&crate::config::HttpSection::default()),
            WebhookMessage {
                sender: "",
                message: "",
                datetime: "",
            },
            &profile,
        )
        .await;
        server.abort();

        assert_eq!(
            outcome,
            ForwardOutcome::PermanentFailure("http_status_302".to_string())
        );
    }
}
