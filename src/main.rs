mod api;
mod assets;
mod cli;
mod config;
mod dbus;
mod events;
mod forward;
mod message;
mod runtime;
mod smscode;
mod storage;
mod util;
mod wizard;

use std::path::Path;

use anyhow::Result;
use clap::Parser;
use cli::{Args, Command, ConfigCommand};
use is_terminal::IsTerminal;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let args = Args::parse();

    match args.command {
        None => {
            if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
                let existing = config::AppConfig::load(&args.config).ok();
                if let Some(cfg) = wizard::run_setup_wizard(existing)? {
                    wizard::write_setup_config(&cfg, &args.config)?;
                    print_setup_next_steps(&args.config);
                } else {
                    println!("existing config kept: {}", args.config.display());
                }
                Ok(())
            } else {
                Err(anyhow::anyhow!(
                    "no subcommand in non-interactive mode; run `sms-relayed run --config {}` or `sms-relayed setup`",
                    args.config.display()
                ))
            }
        }
        Some(Command::Setup) => {
            let existing = config::AppConfig::load(&args.config).ok();
            if let Some(cfg) = wizard::run_setup_wizard(existing)? {
                wizard::write_setup_config(&cfg, &args.config)?;
                print_setup_next_steps(&args.config);
            } else {
                println!("existing config kept: {}", args.config.display());
            }
            Ok(())
        }
        Some(Command::Run) => runtime::run_forwarding(&args.config).await,
        Some(Command::Send) => runtime::send_interactive(&args.config).await,
        Some(Command::Config { command }) => match command {
            ConfigCommand::Check => {
                let cfg = config::AppConfig::load(&args.config)?;
                cfg.validate()?;
                println!("config ok: {}", args.config.display());
                Ok(())
            }
            ConfigCommand::Show => {
                let cfg = config::AppConfig::load(&args.config)?;
                cfg.validate()?;
                println!("{}", cfg.redacted_summary());
                Ok(())
            }
        },
    }
}

fn print_setup_next_steps(config_path: &Path) {
    println!("config written: {}", config_path.display());
    println!(
        "check config: sms-relayed --config {} config check",
        config_path.display()
    );
    println!("start OpenWrt service: /etc/init.d/sms-relayed start");
    println!("inspect OpenWrt logs: logread | tail -n 50");
}
