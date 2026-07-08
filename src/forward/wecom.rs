use anyhow::Result;
use log::{error, info};

use crate::config::{AppConfig, WeComConfig};
use crate::smscode;

pub async fn send(
    tel_number: &str,
    sms_text: &str,
    sms_date: &str,
    device_name: &str,
    profile: &WeComConfig,
    app_config: &AppConfig,
) -> Result<()> {
    let corpid = profile.corp_id.as_str();
    let corpsecret = profile.secret.as_str();
    let agentid: i64 = profile
        .agent_id
        .parse()
        .map_err(|_| anyhow::anyhow!("agent_id format error"))?;

    let mut content = format!(
        "短信转发\n发信电话:{}\n时间:{}\n转发设备:{}\n短信内容:{}",
        tel_number, sms_date, device_name, sms_text
    );
    let (code_str, _, _) = smscode::get_sms_code_str(sms_text, app_config);
    if !code_str.is_empty() {
        content = format!("{}\n{}", code_str, content);
    }

    let client = reqwest::Client::new();

    // Step 1: Get access token
    let token_url = format!(
        "https://qyapi.weixin.qq.com/cgi-bin/gettoken?corpid={}&corpsecret={}",
        corpid, corpsecret
    );
    let token_resp = client.get(&token_url).send().await?;
    let token_json: serde_json::Value = token_resp.json().await?;
    if token_json["errcode"].as_i64() != Some(0) {
        error!(
            "企业微信获取token失败: {}",
            token_json["errmsg"].as_str().unwrap_or("unknown")
        );
        return Ok(());
    }
    let access_token = token_json["access_token"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("access_token为空"))?;

    // Step 2: Send message
    let msg_url = format!(
        "https://qyapi.weixin.qq.com/cgi-bin/message/send?access_token={}",
        access_token
    );
    let msg_body = serde_json::json!({
        "touser": profile.to_user,
        "toparty": "",
        "totag": "",
        "msgtype": "text",
        "agentid": agentid,
        "text": {
            "content": content
        },
        "safe": 0,
        "enable_id_trans": 0,
        "enable_duplicate_check": 0,
        "duplicate_check_interval": 1800
    });

    let msg_resp = client.post(&msg_url).json(&msg_body).send().await?;
    let msg_json: serde_json::Value = msg_resp.json().await?;
    if msg_json["errcode"].as_i64() == Some(0) && msg_json["errmsg"].as_str() == Some("ok") {
        info!("企业微信转发成功");
    } else {
        error!(
            "企业微信转发失败: {}",
            msg_json["errmsg"].as_str().unwrap_or("unknown")
        );
    }
    Ok(())
}
