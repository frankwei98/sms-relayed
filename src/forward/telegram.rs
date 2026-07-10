use log::{error, info};

use crate::config::{AppConfig, TelegramConfig};
use crate::forward::ForwardOutcome;
use crate::smscode;
use crate::util;

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

    let url = format!(
        "{}/bot{}/sendMessage?chat_id={}&text={}",
        base_url,
        token,
        chat_id,
        util::url_encode_form(&text)
    );

    let resp = match client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => return ForwardOutcome::TransientFailure(e.to_string()),
    };
    let json: serde_json::Value = match resp.json().await {
        Ok(j) => j,
        Err(e) => return ForwardOutcome::TransientFailure(e.to_string()),
    };
    if json["ok"].as_bool() == Some(true) {
        info!("TGBot转发成功");
        ForwardOutcome::Success
    } else {
        let msg = json["description"].as_str().unwrap_or("unknown error");
        error!("TGBot转发失败: {}", msg);
        ForwardOutcome::PermanentFailure(msg.to_string())
    }
}
