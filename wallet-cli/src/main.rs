mod commands;
mod wallet;

use clap::Parser;
use commands::*;

use astram_config::config::Config;

#[derive(Parser)]
#[command(name = "Astram-wallet")]
#[command(about = "Astram CLI Wallet", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Generate => generate_wallet(),
        Commands::GenerateEth => generate_eth_wallet(),
        Commands::Balance { address } => get_balance(&address),
        Commands::Send { to, amount } => {
            let amount_ram = asrm_to_ram(amount);
            println!("Sending {} ASRM to {}", amount, to);
            send_transaction(&to, amount_ram)
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
