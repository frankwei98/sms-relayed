use std::time::Duration;

use log::{error, info};

use crate::config::{AppConfig, ShellConfig};
use crate::forward::ForwardOutcome;
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
) -> ForwardOutcome {
    let shell_path = profile.path.as_str();

    let (_, code, code_from) = smscode::get_sms_code_str(sms_text, app_config);

    let cmd = format!(
        "{} \"{}\" \"{}\" \"{}\" \"{}\" \"{}\" \"{}\"",
        shell_path, tel_number, sms_date, sms_text, code, code_from, device_name
    );
    let status = match runner.run_shell(&cmd, shell_timeout).await {
        Ok(s) => s,
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("shell timeout") {
                return ForwardOutcome::TransientFailure("shell_timeout".to_string());
            }
            return ForwardOutcome::TransientFailure("shell_execution_failed".to_string());
        }
    };

    if status.success() {
        info!("Shell调用成功");
        ForwardOutcome::Success
    } else {
        error!("Shell调用失败");
        ForwardOutcome::PermanentFailure("shell_exit_nonzero".to_string())
    }
}
