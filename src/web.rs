use anyhow::Result;
use axum::extract::{Query, State};
use axum::response::Html;
use axum::routing::get;
use axum::Router;
use log::info;
use serde::Deserialize;

use crate::config::Config;
use crate::dbus;

#[derive(Clone)]
struct AppState {
    dbus_connection: zbus::Connection,
}

#[derive(Deserialize)]
struct SmsParams {
    telnum: String,
    smstext: String,
}

const INDEX_HTML: &str = r#"<!DOCTYPE html>
<html>
<head>
<meta charset="UTF-8">
  <style>
  html, body {
            height: 100%;
            display: flex;
            justify-content: center;
            align-items: center;
            background-color: #f7f7f7;
        }
        .container {
            text-align: center;
            padding: 20px;
            background-color: #fff;
            border-radius: 5px;
            box-shadow: 0 0 10px rgba(0, 0, 0, 0.1);
            width: 90vw;
            max-width: 500px;
        }
        input[type='text'], textarea {
          width: 80%;
            padding: 10px;
            border: 1px solid #ccc;
            border-radius: 5px;
            font-size: 20px;
        }
        button {
            padding: 10px 20px;
            background-color: #007bff;
            color: #fff;
            border: none;
            border-radius: 5px;
            font-size: 20px;
            cursor: pointer;
        }
        button:hover {
            background-color: #0056b3;
        }
</style>
    <title>短信发送</title>
    <script>
        function sendSMS() {
            var phoneNumber = encodeURIComponent(document.getElementById('phone').value);
            var message = encodeURIComponent(document.getElementById('message').value);
			var xhr = new XMLHttpRequest();
			var url=window.location.protocol+'//'+window.location.hostname+':'+window.location.port+'/api?telnum='+phoneNumber+'&smstext='+message;
		    xhr.open('GET', url, true);
		    xhr.onreadystatechange = function ()
			{
				if (xhr.readyState === 4 && xhr.status === 200) {
				  alert('已发送');
				}
		    };
		    xhr.send();
        }
    </script>
</head>
<body>
      <div class='container'>
    <h1>短信发送</h1>
    <form>
        <label for='phone'>收信号码:</label>
        <input type='text' id='phone' name='phone' required><br><br>
        <label for='message'>短信内容:</label>
        <textarea id='message' name='message' required></textarea><br><br>
        <button type='button' onclick='sendSMS()'>发送</button>
    </form>
      </div>
</body>
</html>"#;

async fn index_page() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn send_sms_api(
    State(state): State<AppState>,
    Query(params): Query<SmsParams>,
) -> &'static str {
    if let Err(e) =
        dbus::send_sms(&state.dbus_connection, &params.telnum, &params.smstext, "api").await
    {
        log::error!("API发送短信失败: {}", e);
    }
    "ok"
}

pub async fn start_api_server(config: &Config) -> Result<()> {
    let port: u16 = config
        .get("apiPort")
        .ok_or_else(|| anyhow::anyhow!("apiPort未配置"))?
        .parse()
        .map_err(|_| anyhow::anyhow!("apiPort格式错误"))?;

    println!("短信发送 Web API 正在启动，正在连接系统 D-Bus。");
    let connection = zbus::Connection::system().await?;
    let state = AppState {
        dbus_connection: connection,
    };

    let app = Router::new()
        .route("/", get(index_page))
        .route("/api", get(send_sms_api))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port)).await?;
    println!("短信发送 Web API 已运行在 {} 端口。", port);
    info!("短信发送webapi接口已运行在{}端口", port);
    axum::serve(listener, app).await?;
    Ok(())
}
