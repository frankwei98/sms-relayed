mod cli;
mod config;
mod dbus;
mod forward;
mod mode;
mod setup;
mod smscode;
mod util;
mod web;

use std::io::{self, Write};

use anyhow::Result;
use clap::Parser;

use cli::{Args, Channel, RunMode};
use config::Config;

fn preprocess_args() -> Vec<String> {
    let raw: Vec<String> = std::env::args().collect();
    let mut out = Vec::with_capacity(raw.len());
    for a in &raw {
        match a.as_str() {
            "-fP" => out.push("--fP".into()),
            "-fW" => out.push("--fW".into()),
            "-fT" => out.push("--fT".into()),
            "-fD" => out.push("--fD".into()),
            "-fB" => out.push("--fB".into()),
            "-fS" => out.push("--fS".into()),
            "-sS" => out.push("--sS".into()),
            _ => out.push(a.clone()),
        }
    }
    out
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let preprocessed = preprocess_args();
    let args = Args::parse_from(preprocessed);
    let (run_mode, send_method) = args.resolve();
    let config_path = args.config_file.as_deref();

    match run_mode {
        RunMode::Forward { channels, with_api } => {
            Config::check_and_create(config_path)?;
            let mut config = Config::load(config_path)?;

            if channels.is_empty() {
                // No channels specified via CLI, need interactive selection
                let mode_choice = mode::interactive_mode_select();
                match mode_choice.as_str() {
                    "1" => {
                        setup::setup_device_name(&mut config, "");
                        let channel_choice = send_method
                            .clone()
                            .unwrap_or_else(|| mode::interactive_channel_select());
                        let channels = resolve_channels(&channel_choice, &mut config)?;
                        dbus::monitor_dbus(&channels, &config).await?;
                    }
                    "2" => {
                        let connection = zbus::Connection::system().await?;
                        print!("请输入收信号码：\n");
                        io::stdout().flush()?;
                        let mut tel_number = String::new();
                        io::stdin().read_line(&mut tel_number)?;
                        print!("请输入短信内容\n");
                        io::stdout().flush()?;
                        let mut sms_text = String::new();
                        io::stdin().read_line(&mut sms_text)?;
                        dbus::send_sms(&connection, tel_number.trim(), sms_text.trim(), "command")
                            .await?;
                    }
                    "3" => {
                        setup::setup_device_name(&mut config, "*Host*Name*");
                        setup::setup_api_port(&mut config);
                        let channel_choice = send_method
                            .clone()
                            .unwrap_or_else(|| mode::interactive_channel_select());
                        let channels = resolve_channels(&channel_choice, &mut config)?;
                        let config_clone = config.clone();
                        let api_handle = tokio::spawn(async move {
                            if let Err(e) = web::start_api_server(&config_clone).await {
                                log::error!("Web API服务器错误: {}", e);
                            }
                        });
                        let config_clone2 = config.clone();
                        let fwd_handle = tokio::spawn(async move {
                            if let Err(e) = dbus::monitor_dbus(&channels, &config_clone2).await {
                                log::error!("D-Bus监控错误: {}", e);
                            }
                        });
                        tokio::try_join!(api_handle, fwd_handle)?;
                    }
                    "4" => {
                        setup::setup_api_port(&mut config);
                        web::start_api_server(&config).await?;
                    }
                    _ => {
                        return Err(anyhow::anyhow!("无效的运行模式"));
                    }
                }
            } else {
                // Channels specified via CLI
                setup::setup_device_name(&mut config, "*Host*Name*");
                for ch in &channels {
                    let ch_str = match ch {
                        Channel::PushPlus => "1",
                        Channel::WeCom => "2",
                        Channel::Telegram => "3",
                        Channel::DingTalk => "4",
                        Channel::Bark => "5",
                        Channel::Shell => "6",
                    };
                    setup::setup_channel(&mut config, ch_str);
                }

                if with_api {
                    setup::setup_api_port(&mut config);
                    let config_clone = config.clone();
                    let channels_clone = channels.clone();
                    let api_handle = tokio::spawn(async move {
                        if let Err(e) = web::start_api_server(&config_clone).await {
                            log::error!("Web API服务器错误: {}", e);
                        }
                    });
                    let config_clone2 = config.clone();
                    let fwd_handle = tokio::spawn(async move {
                        if let Err(e) = dbus::monitor_dbus(&channels_clone, &config_clone2).await {
                            log::error!("D-Bus监控错误: {}", e);
                        }
                    });
                    tokio::try_join!(api_handle, fwd_handle)?;
                } else {
                    dbus::monitor_dbus(&channels, &config).await?;
                }
            }
        }
        RunMode::SendSms => {
            let connection = zbus::Connection::system().await?;
            print!("请输入收信号码：\n");
            io::stdout().flush()?;
            let mut tel_number = String::new();
            io::stdin().read_line(&mut tel_number)?;
            print!("请输入短信内容\n");
            io::stdout().flush()?;
            let mut sms_text = String::new();
            io::stdin().read_line(&mut sms_text)?;
            dbus::send_sms(&connection, tel_number.trim(), sms_text.trim(), "command").await?;
        }
        RunMode::ApiOnly => {
            Config::check_and_create(config_path)?;
            let mut config = Config::load(config_path)?;
            setup::setup_api_port(&mut config);
            web::start_api_server(&config).await?;
        }
    }
    Ok(())
}

fn resolve_channels(channel_str: &str, config: &mut Config) -> Result<Vec<Channel>> {
    if channel_str == "7" {
        let choices = mode::interactive_multi_channel_select();
        for choice in &choices {
            setup::setup_channel(config, choice);
        }
        Ok(choices.iter().filter_map(|c| str_to_channel(c)).collect())
    } else {
        setup::setup_channel(config, channel_str);
        Ok(str_to_channel(channel_str).into_iter().collect())
    }
}

fn str_to_channel(s: &str) -> Option<Channel> {
    match s {
        "1" => Some(Channel::PushPlus),
        "2" => Some(Channel::WeCom),
        "3" => Some(Channel::Telegram),
        "4" => Some(Channel::DingTalk),
        "5" => Some(Channel::Bark),
        "6" => Some(Channel::Shell),
        _ => None,
    }
}
