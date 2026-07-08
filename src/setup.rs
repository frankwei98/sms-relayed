use std::io::{self, Write};

use crate::config::Config;

fn prompt(msg: &str) -> String {
    print!("{}", msg);
    io::stdout().flush().unwrap();
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    input.trim().to_string()
}

pub fn setup_device_name(config: &mut Config, default_name: &str) {
    if config.get_or_empty("ForwardDeviceName").is_empty() {
        let name = if !default_name.is_empty() {
            default_name.to_string()
        } else {
            let input =
                prompt("初次运行是否需要设置转发设备名称?(留空回车则默认动态读取设备主机名)：\n");
            if input.is_empty() {
                "*Host*Name*".to_string()
            } else {
                input
            }
        };
        config.set("ForwardDeviceName", name);
        let _ = config.save(None);
    }
}

pub fn setup_pushplus(config: &mut Config) {
    if config.get_or_empty("pushPlusToken").is_empty() {
        let token = prompt("首次运行请输入PushPlusToken\n");
        config.set("pushPlusToken", token);
        let _ = config.save(None);
    }
}

pub fn setup_wecom(config: &mut Config) {
    if config.get_or_empty("WeChatQYID").is_empty()
        && config.get_or_empty("WeChatQYApplicationSecret").is_empty()
        && config.get_or_empty("WeChatQYApplicationID").is_empty()
    {
        let corpid = prompt("首次运行请输入企业ID\n");
        let appid = prompt("请输入自建应用ID\n");
        let secret = prompt("请输入自建应用密钥\n");
        config.set("WeChatQYID", corpid);
        config.set("WeChatQYApplicationID", appid);
        config.set("WeChatQYApplicationSecret", secret);
        let _ = config.save(None);
    }
}

pub fn setup_tg_bot(config: &mut Config) {
    if config.get_or_empty("TGBotToken").is_empty()
        && config.get_or_empty("TGBotChatID").is_empty()
        && config.get_or_empty("IsEnableCustomTGBotApi").is_empty()
    {
        let token = prompt("首次运行请输入TG机器人Token\n");
        let chat_id = prompt("请输入机器人要转发到的ChatId\n");
        config.set("TGBotToken", token);
        config.set("TGBotChatID", chat_id);
        loop {
            let choice = prompt("是否需要使用自定义api(1.使用 2.不使用)\n");
            if choice == "1" {
                config.set("IsEnableCustomTGBotApi", "true".to_string());
                let api = prompt("请输入机器人自定义api(格式https://xxx.abc.com)\n");
                config.set("CustomTGBotApi", api);
                break;
            } else if choice == "2" {
                config.set("IsEnableCustomTGBotApi", "false".to_string());
                break;
            }
        }
        let _ = config.save(None);
    }
}

pub fn setup_dingtalk(config: &mut Config) {
    if config.get_or_empty("DingTalkAccessToken").is_empty()
        && config.get_or_empty("DingTalkSecret").is_empty()
    {
        let token = prompt("首次运行请输入钉钉机器人AccessToken\n");
        let secret = prompt("请输入钉钉机器人加签secret\n");
        config.set("DingTalkAccessToken", token);
        config.set("DingTalkSecret", secret);
        let _ = config.save(None);
    }
}

pub fn setup_bark(config: &mut Config) {
    if config.get_or_empty("BarkUrl").is_empty() && config.get_or_empty("BrakKey").is_empty() {
        let url = prompt("首次运行请输入Bark服务器地址\n");
        let key = prompt("请输入Bark推送key\n");
        config.set("BarkUrl", url);
        config.set("BrakKey", key);
        let _ = config.save(None);
    }
}

pub fn setup_shell(config: &mut Config) {
    if config.get_or_empty("ShellPath").is_empty() {
        let path = prompt("首次运行请输入shell脚本路径\n");
        config.set("ShellPath", path);
        let _ = config.save(None);
    }
}

pub fn setup_api_port(config: &mut Config) {
    if config.get_or_empty("apiPort").is_empty() {
        let port = prompt("首次运行请输入要使用的api端口：\n");
        config.set("apiPort", port);
        let _ = config.save(None);
    }
}

pub fn setup_channel(config: &mut Config, channel: &str) {
    match channel {
        "1" => setup_pushplus(config),
        "2" => setup_wecom(config),
        "3" => setup_tg_bot(config),
        "4" => setup_dingtalk(config),
        "5" => setup_bark(config),
        "6" => setup_shell(config),
        _ => {}
    }
}
