use log::{error, info};

use crate::config::{AppConfig, PushPlusConfig};
use crate::forward::ForwardOutcome;
use crate::smscode;

pub async fn send(
    client: &reqwest::Client,
    tel_number: &str,
    sms_text: &str,
    sms_date: &str,
    device_name: &str,
    profile: &PushPlusConfig,
    app_config: &AppConfig,
) -> ForwardOutcome {
    let token = profile.token.as_str();

    let (code_str, _, _) = smscode::get_sms_code_str(sms_text, app_config);
    let title = if code_str.is_empty() {
        format!("短信转发{}", tel_number)
    } else {
        format!("{} 短信转发{}", code_str, tel_number)
    };
    let content = format!(
        "发信电话:{}\n时间:{}\n转发设备:{}\n短信内容:{}",
        tel_number, sms_date, device_name, sms_text
    );

    let params = [
        ("token", token.to_string()),
        ("title", title),
        ("content", content),
    ];
    let resp = match client
        .post("https://www.pushplus.plus/send")
        .form(&params)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return ForwardOutcome::TransientFailure(e.to_string()),
    };
    let json: serde_json::Value = match resp.json().await {
        Ok(j) => j,
        Err(e) => return ForwardOutcome::TransientFailure(e.to_string()),
    };
    if json["code"].as_i64() == Some(200) {
        info!("pushplus转发成功");
        ForwardOutcome::Success
    } else {
        let msg = json["msg"].as_str().unwrap_or("unknown error");
        error!("pushplus转发失败: {}", msg);
        ForwardOutcome::PermanentFailure(msg.to_string())
    }
}
