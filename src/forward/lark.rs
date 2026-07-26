use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use hmac::{Hmac, Mac};
use log::{error, info};
use sha2::Sha256;

use crate::config::{AppConfig, LarkConfig};
use crate::forward::{classify_http_status, transport_failure, ForwardOutcome};
use crate::smscode;

type HmacSha256 = Hmac<Sha256>;

const MAX_REQUEST_BYTES: usize = 20 * 1024;

fn sign(secret: &str, timestamp: i64) -> String {
    let string_to_sign = format!("{timestamp}\n{secret}");
    let mac = HmacSha256::new_from_slice(string_to_sign.as_bytes()).expect("HMAC key error");
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

fn classify_lark_rejection(code: Option<i64>) -> ForwardOutcome {
    match code {
        Some(9499) => ForwardOutcome::PermanentFailure("lark_bad_request".to_string()),
        Some(19021) => {
            ForwardOutcome::PermanentFailure("lark_signature_or_timestamp_invalid".to_string())
        }
        Some(19022) => ForwardOutcome::PermanentFailure("lark_ip_not_allowed".to_string()),
        Some(19024) => ForwardOutcome::PermanentFailure("lark_keyword_not_found".to_string()),
        Some(11232) => ForwardOutcome::TransientFailure("lark_rate_limited".to_string()),
        _ => ForwardOutcome::TransientFailure("provider_rejected".to_string()),
    }
}

pub async fn send(
    client: &reqwest::Client,
    tel_number: &str,
    sms_text: &str,
    sms_date: &str,
    device_name: &str,
    profile: &LarkConfig,
    app_config: &AppConfig,
) -> ForwardOutcome {
    let mut content = format!(
        "短信转发\n发信电话:{}\n时间:{}\n转发设备:{}\n短信内容:{}",
        tel_number, sms_date, device_name, sms_text
    );
    let (code_str, _, _) = smscode::get_sms_code_str(sms_text, app_config);
    if !code_str.is_empty() {
        content = format!("{code_str}\n{content}");
    }

    let mut body = serde_json::json!({
        "msg_type": "text",
        "content": {
            "text": content
        }
    });
    if !profile.secret.trim().is_empty() {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        body["timestamp"] = serde_json::Value::String(timestamp.to_string());
        body["sign"] = serde_json::Value::String(sign(&profile.secret, timestamp));
    }

    if serde_json::to_vec(&body).is_ok_and(|encoded| encoded.len() > MAX_REQUEST_BYTES) {
        return ForwardOutcome::PermanentFailure("payload_too_large".to_string());
    }

    let response = match client.post(&profile.webhook_url).json(&body).send().await {
        Ok(response) => response,
        Err(error) => return transport_failure(&error),
    };
    if let Some(outcome) = classify_http_status(response.status()) {
        return outcome;
    }
    let json: serde_json::Value = match response.json().await {
        Ok(json) => json,
        Err(error) => return transport_failure(&error),
    };
    if json["code"].as_i64() == Some(0) {
        info!("Lark转发成功");
        ForwardOutcome::Success
    } else {
        error!("Lark转发失败: provider_rejected");
        classify_lark_rejection(json["code"].as_i64())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::extract::State;
    use axum::routing::post;
    use axum::{Json, Router};

    use super::*;

    #[derive(Clone, Default)]
    struct CaptureState(Arc<Mutex<Option<serde_json::Value>>>);

    async fn capture_request(
        State(state): State<CaptureState>,
        Json(payload): Json<serde_json::Value>,
    ) -> Json<serde_json::Value> {
        *state.0.lock().unwrap() = Some(payload);
        Json(serde_json::json!({ "code": 0, "msg": "success" }))
    }

    #[test]
    fn signing_matches_the_lark_documented_algorithm() {
        assert_eq!(
            sign("demo", 100),
            "jquNHnVOwmDRfw+vqTIrY5dooJAgi5EcRtLsQE4wfXg="
        );
    }

    #[test]
    fn classifies_documented_lark_rejections_with_actionable_safe_codes() {
        let cases = [
            (9499, "lark_bad_request", false),
            (19021, "lark_signature_or_timestamp_invalid", false),
            (19022, "lark_ip_not_allowed", false),
            (19024, "lark_keyword_not_found", false),
            (11232, "lark_rate_limited", true),
        ];

        for (provider_code, expected_code, transient) in cases {
            let outcome = classify_lark_rejection(Some(provider_code));
            match outcome {
                ForwardOutcome::TransientFailure(code) if transient => {
                    assert_eq!(code, expected_code);
                }
                ForwardOutcome::PermanentFailure(code) if !transient => {
                    assert_eq!(code, expected_code);
                }
                other => panic!("unexpected classification for {provider_code}: {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn posts_signed_text_message_to_the_configured_webhook() {
        let state = CaptureState::default();
        let app = Router::new()
            .route("/open-apis/bot/v2/hook/test", post(capture_request))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let outcome = send(
            &reqwest::Client::new(),
            "+15550000000",
            "verification code 123456",
            "2026-07-26T00:00:00Z",
            "router",
            &LarkConfig {
                webhook_url: format!("http://{address}/open-apis/bot/v2/hook/test"),
                secret: "demo".to_string(),
            },
            &AppConfig::default(),
        )
        .await;
        server.abort();

        assert_eq!(outcome, ForwardOutcome::Success);
        let payload = state.0.lock().unwrap().clone().unwrap();
        assert_eq!(payload["msg_type"], "text");
        assert!(payload["content"]["text"]
            .as_str()
            .is_some_and(|text| text.contains("verification code 123456")));
        assert!(payload["timestamp"].as_str().is_some());
        assert!(payload["sign"].as_str().is_some());
    }

    #[tokio::test]
    async fn omits_signature_fields_when_no_secret_is_configured() {
        let state = CaptureState::default();
        let app = Router::new()
            .route("/hook", post(capture_request))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let outcome = send(
            &reqwest::Client::new(),
            "+15550000000",
            "hello",
            "2026-07-26T00:00:00Z",
            "router",
            &LarkConfig {
                webhook_url: format!("http://{address}/hook"),
                secret: String::new(),
            },
            &AppConfig::default(),
        )
        .await;
        server.abort();

        assert_eq!(outcome, ForwardOutcome::Success);
        let payload = state.0.lock().unwrap().clone().unwrap();
        assert!(payload.get("timestamp").is_none());
        assert!(payload.get("sign").is_none());
    }

    #[test]
    fn rejects_payloads_over_the_lark_twenty_kilobyte_limit() {
        let content = "x".repeat(MAX_REQUEST_BYTES);
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let outcome = runtime.block_on(send(
            &reqwest::Client::new(),
            "+15550000000",
            &content,
            "2026-07-26T00:00:00Z",
            "router",
            &LarkConfig {
                webhook_url: "http://127.0.0.1:1/hook".to_string(),
                secret: String::new(),
            },
            &AppConfig::default(),
        ));

        assert_eq!(
            outcome,
            ForwardOutcome::PermanentFailure("payload_too_large".to_string())
        );
    }
}
