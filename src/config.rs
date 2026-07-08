use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Result;

const CONFIG_KEYS: &[(&str, &str)] = &[
    ("pushPlusToken", ""),
    ("WeChatQYID", ""),
    ("WeChatQYApplicationSecret", ""),
    ("WeChatQYApplicationID", ""),
    ("TGBotToken", ""),
    ("TGBotChatID", ""),
    ("IsEnableCustomTGBotApi", ""),
    ("CustomTGBotApi", ""),
    ("DingTalkAccessToken", ""),
    ("DingTalkSecret", ""),
    ("BarkUrl", ""),
    ("BrakKey", ""),
    ("ShellPath", ""),
    ("apiPort", ""),
    ("ForwardDeviceName", ""),
    ("smsCodeKey", "验证码±verification±code±인증±代码±随机码"),
    ("forwardIgnoreStorageType", "sm"),
];

#[derive(Debug, Clone, Default)]
pub struct Config {
    pub data: HashMap<String, String>,
    path: Option<PathBuf>,
}

impl Config {
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let filename = path
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "config.txt".to_string());
        let mut data = HashMap::new();
        if let Ok(content) = fs::read_to_string(&filename) {
            for line in content.lines() {
                if let Some(pos) = line.find('=') {
                    let key = line[..pos].trim().to_string();
                    let value = line[pos + 1..].trim().to_string();
                    data.insert(key, value);
                }
            }
        }
        Ok(Config {
            data,
            path: path.map(Path::to_path_buf),
        })
    }

    pub fn save(&self, path: Option<&Path>) -> Result<()> {
        let filename = path
            .map(Path::to_path_buf)
            .or_else(|| self.path.clone())
            .unwrap_or_else(|| PathBuf::from("config.txt"));
        let mut file = fs::File::create(&filename)?;
        for (key, value) in &self.data {
            writeln!(file, "{} = {}", key, value)?;
        }
        Ok(())
    }

    pub fn check_and_create(path: Option<&Path>) -> Result<()> {
        let filename = path
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "config.txt".to_string());
        if !Path::new(&filename).exists() {
            let mut file = fs::File::create(&filename)?;
            for (key, default_val) in CONFIG_KEYS {
                writeln!(file, "{} = {}", key, default_val)?;
            }
        }
        Ok(())
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.data.get(key).map(|s| s.as_str())
    }

    pub fn get_or_empty(&self, key: &str) -> &str {
        self.data.get(key).map(|s| s.as_str()).unwrap_or("")
    }

    pub fn set(&mut self, key: &str, value: String) {
        self.data.insert(key.to_string(), value);
    }
}

#[cfg(test)]
mod tests {
    use super::Config;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn save_without_path_uses_loaded_config_path() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "sms-relayed-config-test-{}-{}",
            std::process::id(),
            unique
        ));
        fs::create_dir_all(&dir).unwrap();
        let config_path = dir.join("custom-config.txt");
        fs::write(&config_path, "apiPort = \n").unwrap();

        let mut config = Config::load(Some(&config_path)).unwrap();
        config.set("apiPort", "10721".to_string());
        config.save(None).unwrap();

        let content = fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("apiPort = 10721"));

        fs::remove_dir_all(dir).unwrap();
    }
}
