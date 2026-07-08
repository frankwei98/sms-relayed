mod cli;
mod config;
mod dbus;
mod forward;
mod smscode;
mod util;

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
                println!("setup wizard is added in Task 4");
                Ok(())
            } else {
                Err(anyhow::anyhow!(
                    "no subcommand in non-interactive mode; run `sms-relayed run --config {}` or `sms-relayed setup`",
                    args.config.display()
                ))
            }
        }
        Some(Command::Setup) => {
            println!("setup wizard is added in Task 4");
            Ok(())
        }
        Some(Command::Run) => Err(anyhow::anyhow!("runtime is connected in Task 5")),
        Some(Command::Send) => Err(anyhow::anyhow!("send is connected in Task 5")),
        Some(Command::Config { command }) => match command {
            ConfigCommand::Check => Err(anyhow::anyhow!("config check is connected in Task 3")),
            ConfigCommand::Show => Err(anyhow::anyhow!("config show is connected in Task 3")),
        },
    }
}
