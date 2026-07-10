use log::{error, info};

use crate::config::{AppConfig, BarkConfig};
use crate::forward::ForwardOutcome;
use crate::smscode;
use crate::util;

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

    let mut url = format!(
        "{}/{}/{}",
        bark_url,
        bark_key,
        util::url_encode_form(&content)
    );
    if !code_str.is_empty() {
        url.push_str(&format!(
            "?group={}&title={}&autoCopy=1&copy={}",
            tel_number,
            util::url_encode_form(&title),
            code
        ));
    } else {
        url.push_str(&format!("?group={}&title={}", tel_number, title));
    }

    let resp = match client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => return ForwardOutcome::TransientFailure(e.to_string()),
    };
    let json: serde_json::Value = match resp.json().await {
        Ok(j) => j,
        Err(e) => return ForwardOutcome::TransientFailure(e.to_string()),
    };
    if json["code"].as_i64() == Some(200) {
        info!("Bark转发成功");
        ForwardOutcome::Success
    } else {
        let msg = json["message"].as_str().unwrap_or("unknown error");
        error!("Bark转发失败: {}", msg);
        ForwardOutcome::PermanentFailure(msg.to_string())
    }
}
