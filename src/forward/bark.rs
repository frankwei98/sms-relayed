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
    let bark_url = config
        .get("BarkUrl")
        .ok_or_else(|| anyhow::anyhow!("BarkUrl未配置"))?;
    let bark_key = config
        .get("BrakKey")
        .ok_or_else(|| anyhow::anyhow!("BrakKey未配置"))?;

    let (code_str, code, _) = smscode::get_sms_code_str(sms_text, config);
    let title = if code_str.is_empty() {
        format!("短信转发{}", tel_number)
    } else {
        format!("{} 短信转发{}", code_str, tel_number)
    };
    let content = format!(
        "发信电话:{}\n时间:{}\n转发设备:{}\n短信内容:{}",
        tel_number, sms_date, device_name, sms_text
    );

    let mut url = format!("{}/{}/{}", bark_url, bark_key, util::url_encode_form(&content));
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

    let client = reqwest::Client::new();
    let resp = client.get(&url).send().await?;
    let json: serde_json::Value = resp.json().await?;
    if json["code"].as_i64() == Some(200) {
        info!("Bark转发成功");
    } else {
        error!(
            "Bark转发失败: {}",
            json["message"].as_str().unwrap_or("unknown error")
        );
    }
    Ok(())
}
