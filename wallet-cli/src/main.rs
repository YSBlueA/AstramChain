mod commands;
mod wallet;

use clap::Parser;
use commands::*;

use netcoin_config::config::Config;

// NTC unit constants (8 decimal places)
const NATOSHI_PER_NTC: u64 = 100_000_000; // 1 NTC = 100,000,000 natoshi

/// Convert NTC to natoshi (smallest unit)
fn ntc_to_natoshi(ntc: f64) -> u64 {
    (ntc * NATOSHI_PER_NTC as f64) as u64
}

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
        Commands::Send { to, amount } => {
            let amount_natoshi = ntc_to_natoshi(amount);
            println!(
                "📤 Sending {} NTC ({} natoshi) to {}",
                amount, amount_natoshi, to
            );
            send_transaction(&to, amount_natoshi)
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
