use base64::Engine;
use hmac::{Hmac, Mac};
use log::{error, info};
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::{AppConfig, DingTalkConfig};
use crate::forward::{
    classify_http_status, classify_provider_rejection, transport_failure, ForwardOutcome,
};
use crate::smscode;
use crate::util;

type HmacSha256 = Hmac<Sha256>;

fn sign(secret: &str, timestamp_ms: i64) -> String {
    let string_to_sign = format!("{}\n{}", timestamp_ms, secret);
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC key error");
    mac.update(string_to_sign.as_bytes());
    let result = mac.finalize().into_bytes();
    base64::engine::general_purpose::STANDARD.encode(result)
}

pub async fn send(
    client: &reqwest::Client,
    tel_number: &str,
    sms_text: &str,
    sms_date: &str,
    device_name: &str,
    profile: &DingTalkConfig,
    app_config: &AppConfig,
) -> ForwardOutcome {
    let access_token = profile.access_token.as_str();
    let secret = profile.secret.as_str();

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let sign_str = sign(secret, timestamp);
    let url = format!(
        "https://oapi.dingtalk.com/robot/send?access_token={}&timestamp={}&sign={}",
        access_token,
        timestamp,
        util::url_encode_path(&sign_str)
    );

    let mut content = format!(
        "短信转发\n发信电话:{}\n时间:{}\n转发设备:{}\n短信内容:{}",
        tel_number, sms_date, device_name, sms_text
    );
    let (code_str, _, _) = smscode::get_sms_code_str(sms_text, app_config);
    if !code_str.is_empty() {
        content = format!("{}\n{}", code_str, content);
    }

    let body = serde_json::json!({
        "msgtype": "text",
        "text": {
            "content": content
        }
    });

    let resp = match client
        .post(&url)
        .header("Content-Type", "application/json;charset=utf-8")
        .json(&body)
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
    if json["errcode"].as_i64() == Some(0) && json["errmsg"].as_str() == Some("ok") {
        info!("钉钉转发成功");
        ForwardOutcome::Success
    } else {
        error!("钉钉转发失败: provider_rejected");
        classify_provider_rejection(json["errcode"].as_i64(), &[40035, 310000])
    }
}
