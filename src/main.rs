mod api;
mod assets;
mod cli;
mod config;
mod dbus;
mod delivery;
mod events;
mod forward;
mod message;
mod modem;
mod monitoring;
mod runner;
mod runtime;
mod smscode;
mod storage;
mod update;
mod util;
mod wizard;

use std::path::Path;

use anyhow::Result;
use clap::Parser;
use cli::{Args, Command, ConfigCommand};
use config::AppConfig;
use is_terminal::IsTerminal;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let args = Args::parse();
    let is_service = matches!(args.command.as_ref(), Some(Command::Run));
    let _sentry = is_service.then(monitoring::init).flatten();
    let result = run(args).await;
    if is_service && result.is_err() {
        monitoring::capture_failure("process", "process.exit_error");
    }
    result
}

async fn run(args: Args) -> Result<()> {
    match args.command {
        None => {
            if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
                let existing = config::AppConfig::load(&args.config).ok();
                if let Some(cfg) = wizard::run_setup_wizard(existing)? {
                    wizard::write_setup_config(&cfg, &args.config)?;
                    print_setup_next_steps(&args.config, &cfg);
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
                print_setup_next_steps(&args.config, &cfg);
            } else {
                println!("existing config kept: {}", args.config.display());
            }
            Ok(())
        }
        Some(Command::Run) => runtime::run_forwarding(&args.config).await,
        Some(Command::Send) => runtime::send_interactive(&args.config).await,
        Some(Command::Update) => update::run().await,
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

fn print_setup_next_steps(config_path: &Path, cfg: &AppConfig) {
    println!("config written: {}", config_path.display());
    println!(
        "check config: sms-relayed --config {} config check",
        config_path.display()
    );
    println!("start OpenWrt service: /etc/init.d/sms-relayed start");
    if cfg.api.enabled {
        println!("open dashboard: http://<device-ip>:{}", cfg.api.port);
    }
    println!("inspect OpenWrt logs: logread | tail -n 50");
}
