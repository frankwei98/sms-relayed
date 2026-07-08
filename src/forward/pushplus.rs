use anyhow::Result;
use log::{error, info};

use crate::config::{AppConfig, PushPlusConfig};
use crate::smscode;

pub async fn send(
    tel_number: &str,
    sms_text: &str,
    sms_date: &str,
    device_name: &str,
    profile: &PushPlusConfig,
    app_config: &AppConfig,
) -> Result<()> {
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

    let client = reqwest::Client::new();
    let params = [
        ("token", token.to_string()),
        ("title", title),
        ("content", content),
    ];
    let resp = client
        .post("https://www.pushplus.plus/send")
        .form(&params)
        .send()
        .await?;
    let json: serde_json::Value = resp.json().await?;
    if json["code"].as_i64() == Some(200) {
        info!("pushplus转发成功");
    } else {
        error!(
            "pushplus转发失败: {}",
            json["msg"].as_str().unwrap_or("unknown error")
        );
    }
    Ok(())
}
