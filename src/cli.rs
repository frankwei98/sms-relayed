use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Channel {
    PushPlus,
    WeCom,
    Telegram,
    DingTalk,
    Bark,
    Shell,
}

impl Channel {
    pub fn name(&self) -> &str {
        match self {
            Channel::PushPlus => "PushPlus",
            Channel::WeCom => "企业微信",
            Channel::Telegram => "Telegram",
            Channel::DingTalk => "钉钉",
            Channel::Bark => "Bark",
            Channel::Shell => "Shell",
        }
    }
}

#[derive(Debug)]
pub enum RunMode {
    Forward {
        channels: Vec<Channel>,
        with_api: bool,
    },
    SendSms,
    ApiOnly,
}

#[derive(Parser, Debug)]
#[command(name = "DbusSmsForward", version = "1.0.7")]
pub struct Args {
    #[arg(long = "fP")]
    pub forward_pushplus: bool,
    #[arg(long = "fW")]
    pub forward_wecom: bool,
    #[arg(long = "fT")]
    pub forward_tg: bool,
    #[arg(long = "fD")]
    pub forward_dingtalk: bool,
    #[arg(long = "fB")]
    pub forward_bark: bool,
    #[arg(long = "fS")]
    pub forward_shell: bool,
    #[arg(long = "sS")]
    pub send_sms: bool,
    #[arg(long = "configfile")]
    pub config_file: Option<PathBuf>,
    #[arg(long = "sendsmsapi")]
    pub send_sms_api: Option<String>,
}

impl Args {
    pub fn resolve(&self) -> (RunMode, Option<String>) {
        let mut channels = Vec::new();
        let mut send_method = String::new();

        if self.forward_pushplus {
            channels.push(Channel::PushPlus);
            send_method = "1".to_string();
        }
        if self.forward_wecom {
            channels.push(Channel::WeCom);
            send_method = "2".to_string();
        }
        if self.forward_tg {
            channels.push(Channel::Telegram);
            send_method = "3".to_string();
        }
        if self.forward_dingtalk {
            channels.push(Channel::DingTalk);
            send_method = "4".to_string();
        }
        if self.forward_bark {
            channels.push(Channel::Bark);
            send_method = "5".to_string();
        }
        if self.forward_shell {
            channels.push(Channel::Shell);
            send_method = "6".to_string();
        }

        let enable_api = self
            .send_sms_api
            .as_deref()
            .map(|s| s == "enable")
            .unwrap_or(false);

        if !channels.is_empty() {
            if channels.len() > 1 {
                send_method = "7".to_string();
            }
            (
                RunMode::Forward {
                    channels,
                    with_api: enable_api,
                },
                Some(send_method),
            )
        } else if self.send_sms {
            (RunMode::SendSms, None)
        } else if enable_api {
            (RunMode::ApiOnly, None)
        } else {
            // No CLI args, will need interactive mode
            (
                RunMode::Forward {
                    channels: vec![],
                    with_api: false,
                },
                None,
            )
        }
    }
}
