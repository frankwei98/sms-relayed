use anyhow::Result;
use base64::Engine;
use hmac::{Hmac, Mac};
use log::{error, info};
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::Config;
use crate::smscode;
use crate::util;

type HmacSha256 = Hmac<Sha256>;

fn sign(secret: &str, timestamp_ms: i64) -> String {
    let string_to_sign = format!("{}\n{}", timestamp_ms, secret);
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC key error");
    mac.update(string_to_sign.as_bytes());
    let result = mac.finalize().into_bytes();
    base64::engine::general_purpose::STANDARD.encode(&result)
}

pub async fn send(
    tel_number: &str,
    sms_text: &str,
    sms_date: &str,
    device_name: &str,
    config: &Config,
) -> Result<()> {
    let access_token = config
        .get("DingTalkAccessToken")
        .ok_or_else(|| anyhow::anyhow!("DingTalkAccessToken未配置"))?;
    let secret = config
        .get("DingTalkSecret")
        .ok_or_else(|| anyhow::anyhow!("DingTalkSecret未配置"))?;

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
    let (code_str, _, _) = smscode::get_sms_code_str(sms_text, config);
    if !code_str.is_empty() {
        content = format!("{}\n{}", code_str, content);
    }

    let body = serde_json::json!({
        "msgtype": "text",
        "text": {
            "content": content
        }
    });

    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .header("Content-Type", "application/json;charset=utf-8")
        .json(&body)
        .send()
        .await?;
    let json: serde_json::Value = resp.json().await?;
    if json["errcode"].as_i64() == Some(0) && json["errmsg"].as_str() == Some("ok") {
        info!("钉钉转发成功");
    } else {
        error!(
            "钉钉转发失败: {}",
            json["errmsg"].as_str().unwrap_or("unknown")
        );
    }
    Ok(())
}
