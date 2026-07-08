use anyhow::Result;
use log::{error, info};

use crate::config::{AppConfig, ShellConfig};
use crate::smscode;

pub async fn send(
    tel_number: &str,
    sms_text: &str,
    sms_date: &str,
    device_name: &str,
    profile: &ShellConfig,
    app_config: &AppConfig,
) -> Result<()> {
    let shell_path = profile.path.as_str();

    let (_, code, code_from) = smscode::get_sms_code_str(sms_text, app_config);

    let cmd = format!(
        "{} \"{}\" \"{}\" \"{}\" \"{}\" \"{}\" \"{}\"",
        shell_path, tel_number, sms_date, sms_text, code, code_from, device_name
    );
    let status = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .status()
        .await?;

    if status.success() {
        info!("Shell调用成功");
    } else {
        error!("Shell调用失败");
    }
    Ok(())
}
