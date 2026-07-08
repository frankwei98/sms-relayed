use regex::Regex;

use crate::config::Config;

pub fn extract_bracket_content(input: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut start = 0;
    while let Some(open_pos) = input[start..].find('【') {
        let abs_open = start + open_pos;
        if let Some(close_pos) = input[abs_open..].find('】') {
            let abs_close = abs_open + close_pos;
            let content = &input[abs_open + '【'.len_utf8()..abs_close];
            result.push(content.to_string());
            start = abs_close + '】'.len_utf8();
        } else {
            break;
        }
    }
    result
}

pub fn has_verification_keyword(sms_content: &mut String, config: &Config) -> bool {
    let keys_str = config
        .get("smsCodeKey")
        .unwrap_or("验证码±verification±code±인증±代码±随机码");
    let keywords: Vec<&str> = keys_str.split('±').collect();
    for keyword in &keywords {
        if sms_content.contains(keyword) {
            let replacement = format!(" {} ", keyword);
            *sms_content = sms_content.replacen(keyword, &replacement, 1);
            return true;
        }
    }
    false
}

fn count_digits(s: &str) -> usize {
    s.chars().filter(|c| c.is_ascii_digit()).count()
}

pub fn extract_code(sms_content: &str) -> String {
    let re = Regex::new(r"\b[A-Za-z0-9]{4,7}\b").unwrap();
    let matches: Vec<&str> = re.find_iter(sms_content).map(|m| m.as_str()).collect();
    if matches.len() > 1 {
        matches
            .iter()
            .max_by_key(|m| count_digits(m))
            .map(|s| s.to_string())
            .unwrap_or_default()
    } else if matches.len() == 1 {
        matches[0].to_string()
    } else {
        String::new()
    }
}

pub fn extract_code_source(sms_content: &str) -> String {
    let contents = extract_bracket_content(sms_content);
    for content in &contents {
        let parts: Vec<&str> = sms_content.split(content).collect();
        if parts.first().map_or(false, |p| p.ends_with('【'))
            || parts.last().map_or(false, |p| p.starts_with('】'))
        {
            return format!("【{}】", content);
        }
    }
    String::new()
}

pub fn get_sms_code_str(sms_text: &str, config: &Config) -> (String, String, String) {
    let mut content = sms_text.trim().to_string();
    if has_verification_keyword(&mut content, config) {
        let code = extract_code(&content).trim().to_string();
        if !code.is_empty() {
            let source = extract_code_source(&content);
            let combined = format!("{}{}", source, code);
            return (combined, code, source);
        }
    }
    (String::new(), String::new(), String::new())
}
