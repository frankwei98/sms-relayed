use std::path::PathBuf;

use clap::{Parser, Subcommand};

pub const DEFAULT_CONFIG_PATH: &str = "/etc/sms-relayed/config.toml";

#[derive(Parser, Debug)]
#[command(
    name = "sms-relayed",
    version = env!("SMS_RELAYED_BUILD_VERSION"),
    about = "SMS relay for ModemManager devices"
)]
pub struct Args {
    #[arg(long = "config", global = true, default_value = DEFAULT_CONFIG_PATH)]
    pub config: PathBuf,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Setup,
    Run,
    Send,
    Update,
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum ConfigCommand {
    Check,
    Show,
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser};

    use super::{Args, Command};

    #[test]
    fn version_uses_build_metadata() {
        let command = Args::command();
        let version = command.get_version().expect("version should be set");

        assert_eq!(version, env!("SMS_RELAYED_BUILD_VERSION"));
        assert!(version.starts_with(concat!(env!("CARGO_PKG_VERSION"), "+")));
    }

    #[test]
    fn update_subcommand_is_available() {
        let args = Args::try_parse_from(["sms-relayed", "update"]).expect("update should parse");

        assert_eq!(args.command, Some(Command::Update));
    }
}
