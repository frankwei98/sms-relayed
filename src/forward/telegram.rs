use log::{error, info};

use crate::config::{AppConfig, TelegramConfig};
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
    profile: &TelegramConfig,
    app_config: &AppConfig,
) -> ForwardOutcome {
    let token = profile.bot_token.as_str();
    let chat_id = profile.chat_id.as_str();
    let base_url = profile.api_base.trim_end_matches('/');

    let content = format!(
        "发信电话:{}\n时间:{}\n转发设备:{}\n短信内容:{}",
        tel_number, sms_date, device_name, sms_text
    );
    let (code_str, _, _) = smscode::get_sms_code_str(sms_text, app_config);
    let text = if code_str.is_empty() {
        format!("短信转发\n{}", content)
    } else {
        format!("{}\n短信转发\n{}", code_str, content)
    };

    let url = format!("{base_url}/bot{token}/sendMessage");
    let form = [("chat_id", chat_id), ("text", text.as_str())];

    let resp = match client.post(&url).form(&form).send().await {
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
    if json["ok"].as_bool() == Some(true) {
        info!("TGBot转发成功");
        ForwardOutcome::Success
    } else {
        error!("TGBot转发失败: provider_rejected");
        classify_provider_rejection(json["error_code"].as_i64(), &[400, 401, 403, 404])
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use axum::extract::{Form, State};
    use axum::routing::post;
    use axum::{Json, Router};

    use super::*;

    #[derive(Clone, Default)]
    struct CaptureState(Arc<Mutex<Option<HashMap<String, String>>>>);

    async fn capture_request(
        State(state): State<CaptureState>,
        Form(payload): Form<HashMap<String, String>>,
    ) -> Json<serde_json::Value> {
        *state.0.lock().unwrap() = Some(payload);
        Json(serde_json::json!({ "ok": true }))
    }

    #[tokio::test]
    async fn sends_message_content_in_post_body() {
        let state = CaptureState::default();
        let app = Router::new()
            .route("/botsecret/sendMessage", post(capture_request))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let body = "verification code 123456";

        let outcome = send(
            &reqwest::Client::new(),
            "+15550000000",
            body,
            "2026-07-24T00:00:00Z",
            "router",
            &TelegramConfig {
                bot_token: "secret".to_string(),
                chat_id: "-10001".to_string(),
                api_base: format!("http://{address}"),
            },
            &AppConfig::default(),
        )
        .await;
        server.abort();

        assert_eq!(outcome, ForwardOutcome::Success);
        let payload = state.0.lock().unwrap().clone().unwrap();
        assert_eq!(payload.get("chat_id").map(String::as_str), Some("-10001"));
        assert!(payload.get("text").is_some_and(|text| text.contains(body)));
    }
}
