use std::time::Duration;

use log::{error, info};

use crate::config::{AppConfig, ShellConfig};
use crate::forward::ForwardOutcome;
use crate::runner::ProcessRunner;
use crate::smscode;

pub struct ShellMessage<'a> {
    pub tel_number: &'a str,
    pub sms_text: &'a str,
    pub sms_date: &'a str,
    pub device_name: &'a str,
}

pub async fn send(
    runner: &dyn ProcessRunner,
    shell_timeout: Duration,
    message: ShellMessage<'_>,
    profile: &ShellConfig,
    app_config: &AppConfig,
) -> ForwardOutcome {
    let shell_path = profile.path.as_str();

    let (_, code, code_from) = smscode::get_sms_code_str(message.sms_text, app_config);

    let arguments = [
        message.tel_number,
        message.sms_date,
        message.sms_text,
        code.as_str(),
        code_from.as_str(),
        message.device_name,
    ];
    let status = match runner
        .run_command(shell_path, &arguments, shell_timeout)
        .await
    {
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

#[cfg(all(test, unix))]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    use uuid::Uuid;

    use super::*;
    use crate::runner::RealProcessRunner;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("sms-relayed-shell-test-{}", Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[tokio::test]
    async fn sms_body_is_passed_to_the_script_as_one_literal_argument() {
        let directory = TestDirectory::new();
        let script = directory.path().join("capture-body.sh");
        let output = directory.path().join("body.txt");
        let marker = directory.path().join("must-not-exist");
        fs::write(&script, "#!/bin/sh\nprintf '%s' \"$3\" > \"$1\"\n").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

        let body = format!("message\"; touch {}; echo \"", marker.display());
        let outcome = send(
            &RealProcessRunner,
            Duration::from_secs(5),
            ShellMessage {
                tel_number: output.to_str().unwrap(),
                sms_text: &body,
                sms_date: "2026-07-23T00:00:00Z",
                device_name: "device",
            },
            &ShellConfig {
                path: script.display().to_string(),
            },
            &AppConfig::default(),
        )
        .await;

        assert_eq!(outcome, ForwardOutcome::Success);
        assert_eq!(fs::read_to_string(output).unwrap(), body);
        assert!(
            !marker.exists(),
            "an SMS body must not be interpreted as a shell command"
        );
    }
}
