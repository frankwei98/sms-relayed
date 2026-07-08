use std::io::{self, Write};

/// Interactive mode selection. Returns "1", "2", "3", or "4".
pub fn interactive_mode_select() -> String {
    print!("请选择运行模式：1为短信转发模式，2为发短信模式，3为短信转发模式并开启发短信webapi接口，4为只运行发短信webapi接口\n");
    io::stdout().flush().unwrap();
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    let choice = input.trim().to_string();
    match choice.as_str() {
        "1" | "2" | "3" | "4" => choice,
        _ => {
            println!("请输入1或2或3或4");
            interactive_mode_select()
        }
    }
}

/// Interactive channel selection. Returns "1"-"7".
pub fn interactive_channel_select() -> String {
    print!("请选择转发渠道：1.pushplus转发，2.企业微信转发，3.TG机器人转发，4.钉钉转发，5.Bark转发，6.Shell脚本转发，7.自选多渠道转发\n");
    io::stdout().flush().unwrap();
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    let choice = input.trim().to_string();
    match choice.as_str() {
        "1" | "2" | "3" | "4" | "5" | "6" | "7" => choice,
        _ => {
            println!("请输入1或2或3或4或5或6或7");
            interactive_channel_select()
        }
    }
}

/// Interactive multi-channel selection. Returns Vec of "1"-"6".
pub fn interactive_multi_channel_select() -> Vec<String> {
    print!("请正确输入需要使用的转发渠道编号，以空格分隔（举例：1 2 3 5）\n");
    io::stdout().flush().unwrap();
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    let choices: Vec<String> = input
        .trim()
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();
    let valid = choices
        .iter()
        .all(|c| matches!(c.as_str(), "1" | "2" | "3" | "4" | "5" | "6"));
    if valid && !choices.is_empty() {
        let mut choices = choices;
        crate::util::dedup(&mut choices);
        choices
    } else {
        println!("请输入正确的渠道编号");
        interactive_multi_channel_select()
    }
}
