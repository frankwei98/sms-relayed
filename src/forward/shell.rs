use std::time::Duration;

use anyhow::Result;
use log::{error, info};

use crate::config::{AppConfig, ShellConfig};
use crate::runner::ProcessRunner;
use crate::smscode;

pub async fn send(
    runner: &dyn ProcessRunner,
    shell_timeout: Duration,
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
    let status = runner.run_shell(&cmd, shell_timeout).await?;

    if status.success() {
        info!("Shell调用成功");
    } else {
        error!("Shell调用失败");
    }
    Ok(())
}
