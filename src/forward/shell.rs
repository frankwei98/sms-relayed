use anyhow::Result;
use log::{error, info};

use crate::config::Config;
use crate::smscode;

pub async fn send(
    tel_number: &str,
    sms_text: &str,
    sms_date: &str,
    device_name: &str,
    config: &Config,
) -> Result<()> {
    let shell_path = config
        .get("ShellPath")
        .ok_or_else(|| anyhow::anyhow!("ShellPath未配置"))?;

    let (_, code, code_from) = smscode::get_sms_code_str(sms_text, config);

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
