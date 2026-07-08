use std::path::PathBuf;

use clap::{Parser, Subcommand};

pub const DEFAULT_CONFIG_PATH: &str = "/etc/sms-relayed/config.toml";

#[derive(Parser, Debug)]
#[command(name = "sms-relayed", version, about = "SMS relay for ModemManager devices")]
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


