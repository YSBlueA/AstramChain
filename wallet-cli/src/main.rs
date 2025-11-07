mod wallet;
mod commands;

use clap::{Parser, Subcommand};
use commands::*;

use netcoin_config::config::Config;

#[derive(Parser)]
#[command(name = "netcoin-wallet")]
#[command(about = "NetCoin CLI Wallet", long_about = None)]
struct Cli {
#[command(subcommand)]
    command: Commands,
}


fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Generate => generate_wallet(),
        Commands::Balance { address } => get_balance(&address),
        Commands::Send { from, to, amount, private_key } => {
            send_transaction(&from, &to, amount, &private_key)
        }
        Commands::Config { subcommand } => match subcommand {
            ConfigCommands::View => {
                let cfg = Config::load();
                cfg.view();
            }
            ConfigCommands::Set { key, value } => {
                let mut cfg = Config::load();
                cfg.set_value(&key, &value);
            }
            ConfigCommands::Init => {
                Config::init_default();
            }
        },
    }
}