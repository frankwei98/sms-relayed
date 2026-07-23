use log::{error, info};

use crate::config::{AppConfig, BarkConfig};
use crate::forward::{
    classify_http_status, classify_provider_rejection, transport_failure, ForwardOutcome,
};
use crate::smscode;

pub async fn send(
    client: &reqwest::Client,
    tel_number: &str,
    sms_text: &str,
    sms_date: &str,
    device_name: &str,
    profile: &BarkConfig,
    app_config: &AppConfig,
) -> ForwardOutcome {
    let bark_url = profile.server_url.trim_end_matches('/');
    let bark_key = profile.key.as_str();

    let (code_str, code, _) = smscode::get_sms_code_str(sms_text, app_config);
    let title = if code_str.is_empty() {
        format!("短信转发{}", tel_number)
    } else {
        format!("{} 短信转发{}", code_str, tel_number)
    };
    let content = format!(
        "发信电话:{}\n时间:{}\n转发设备:{}\n短信内容:{}",
        tel_number, sms_date, device_name, sms_text
    );

    let mut payload = serde_json::json!({
        "body": content,
        "device_key": bark_key,
        "group": tel_number,
        "title": title,
    });
    if !code_str.is_empty() {
        payload["autoCopy"] = serde_json::Value::String("1".to_string());
        payload["copy"] = serde_json::Value::String(code);
    }

    let resp = match client
        .post(format!("{bark_url}/push"))
        .json(&payload)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return transport_failure(&e),
    };
    if let Some(outcome) = classify_http_status(resp.status()) {
        return outcome;
    }
    let json: serde_json::Value = match resp.json().await {
        Ok(j) => j,
        Err(e) => return transport_failure(&e),
    };
    if json["code"].as_i64() == Some(200) {
        info!("Bark转发成功");
        ForwardOutcome::Success
    } else {
        error!("Bark转发失败: provider_rejected");
        classify_provider_rejection(json["code"].as_i64(), &[400, 401, 403, 404, 422])
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::extract::State;
    use axum::routing::post;
    use axum::{Json, Router};

    use super::*;

    type CapturedRequest = serde_json::Value;

    #[derive(Clone, Default)]
    struct CaptureState(Arc<Mutex<Option<CapturedRequest>>>);

    async fn capture_request(
        State(state): State<CaptureState>,
        Json(payload): Json<serde_json::Value>,
    ) -> Json<serde_json::Value> {
        *state.0.lock().unwrap() = Some(payload);
        Json(serde_json::json!({ "code": 200 }))
    }

    #[tokio::test]
    async fn bark_v2_posts_json_to_push_endpoint() {
        let state = CaptureState::default();
        let app = Router::new()
            .route("/push", post(capture_request))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let phone_number = "+1555 & title=unexpected";
        let sms_body = "验证码是 123456，正文有空格 + plus & ampersand";
        let outcome = send(
            &reqwest::Client::new(),
            phone_number,
            sms_body,
            "2026-07-23T00:00:00Z",
            "router",
            &BarkConfig {
                server_url: format!("http://{address}/"),
                key: "device key".to_string(),
            },
            &AppConfig::default(),
        )
        .await;
        server.abort();

        assert_eq!(outcome, ForwardOutcome::Success);
        let payload = state.0.lock().unwrap().clone().unwrap();
        assert_eq!(payload["device_key"], "device key");
        assert_eq!(
            payload["body"],
            format!(
                "发信电话:{phone_number}\n时间:2026-07-23T00:00:00Z\n转发设备:router\n短信内容:{sms_body}"
            )
        );
        assert_eq!(payload["group"], phone_number);
        assert_eq!(payload["title"], format!("123456 短信转发{phone_number}"));
        assert_eq!(payload["autoCopy"], "1");
        assert_eq!(payload["copy"], "123456");
    }
}
