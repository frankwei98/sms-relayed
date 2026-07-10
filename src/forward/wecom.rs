use log::{error, info};

use crate::config::{AppConfig, WeComConfig};
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
    profile: &WeComConfig,
    app_config: &AppConfig,
) -> ForwardOutcome {
    let corpid = profile.corp_id.as_str();
    let corpsecret = profile.secret.as_str();
    let agentid: i64 = match profile.agent_id.parse() {
        Ok(id) => id,
        Err(_) => return ForwardOutcome::PermanentFailure("invalid_agent_id".to_string()),
    };

    let mut content = format!(
        "短信转发\n发信电话:{}\n时间:{}\n转发设备:{}\n短信内容:{}",
        tel_number, sms_date, device_name, sms_text
    );
    let (code_str, _, _) = smscode::get_sms_code_str(sms_text, app_config);
    if !code_str.is_empty() {
        content = format!("{}\n{}", code_str, content);
    }

    // Step 1: Get access token
    let token_url = format!(
        "https://qyapi.weixin.qq.com/cgi-bin/gettoken?corpid={}&corpsecret={}",
        corpid, corpsecret
    );
    let token_resp = match client.get(&token_url).send().await {
        Ok(r) => r,
        Err(e) => return transport_failure(&e),
    };
    if let Some(outcome) = classify_http_status(token_resp.status()) {
        return outcome;
    }
    let token_json: serde_json::Value = match token_resp.json().await {
        Ok(j) => j,
        Err(e) => return transport_failure(&e),
    };
    if token_json["errcode"].as_i64() != Some(0) {
        error!("企业微信获取token失败: provider_rejected");
        return classify_provider_rejection(
            token_json["errcode"].as_i64(),
            &[40013, 40014, 41001, 41002],
        );
    }
    let access_token = match token_json["access_token"].as_str() {
        Some(t) => t.to_string(),
        None => return ForwardOutcome::TransientFailure("provider_malformed_response".to_string()),
    };

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

    let msg_resp = match client.post(&msg_url).json(&msg_body).send().await {
        Ok(r) => r,
        Err(e) => return transport_failure(&e),
    };
    if let Some(outcome) = classify_http_status(msg_resp.status()) {
        return outcome;
    }
    let msg_json: serde_json::Value = match msg_resp.json().await {
        Ok(j) => j,
        Err(e) => return transport_failure(&e),
    };
    if msg_json["errcode"].as_i64() == Some(0) && msg_json["errmsg"].as_str() == Some("ok") {
        info!("企业微信转发成功");
        ForwardOutcome::Success
    } else {
        error!("企业微信转发失败: provider_rejected");
        classify_provider_rejection(
            msg_json["errcode"].as_i64(),
            &[40014, 40056, 41001, 41009, 48002, 60011, 60111],
        )
    }
}
