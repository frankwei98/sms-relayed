use anyhow::Result;
use log::{error, info};

use crate::config::Config;
use crate::smscode;
use crate::util;

pub async fn send(
    tel_number: &str,
    sms_text: &str,
    sms_date: &str,
    device_name: &str,
    config: &Config,
) -> Result<()> {
    let token = config
        .get("TGBotToken")
        .ok_or_else(|| anyhow::anyhow!("TGBotToken未配置"))?;
    let chat_id = config
        .get("TGBotChatID")
        .ok_or_else(|| anyhow::anyhow!("TGBotChatID未配置"))?;

    let base_url = if config.get_or_empty("IsEnableCustomTGBotApi") == "true" {
        config.get_or_empty("CustomTGBotApi").to_string()
    } else {
        "https://api.telegram.org".to_string()
    };

    let content = format!(
        "发信电话:{}\n时间:{}\n转发设备:{}\n短信内容:{}",
        tel_number, sms_date, device_name, sms_text
    );
    let (code_str, _, _) = smscode::get_sms_code_str(sms_text, config);
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

    let client = reqwest::Client::new();
    let resp = client.get(&url).send().await?;
    let json: serde_json::Value = resp.json().await?;
    if json["ok"].as_bool() == Some(true) {
        info!("TGBot转发成功");
    } else {
        error!(
            "TGBot转发失败: {}",
            json["description"].as_str().unwrap_or("unknown error")
        );
    }
    Ok(())
}
