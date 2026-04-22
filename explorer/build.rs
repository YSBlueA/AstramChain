use std::path::PathBuf;

fn main() {
    // Create config/explorerSettings.conf next to the binary output dir.
    // We write it relative to CARGO_MANIFEST_DIR (the explorer package root)
    // so it appears at explorer/config/explorerSettings.conf in the repo,
    // and also copy it to the release output dir when building.
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let config_dir = PathBuf::from(&manifest_dir).join("config");
    let conf_path = config_dir.join("explorerSettings.conf");

    if !config_dir.exists() {
        std::fs::create_dir_all(&config_dir).expect("Failed to create config directory");
    }

    if !conf_path.exists() {
        std::fs::write(&conf_path, DEFAULT_CONF).expect("Failed to write explorerSettings.conf");
        println!("cargo:warning=Created default config: {:?}", conf_path);
    }

    // Also write to the OUT_DIR so build scripts can reference it,
    // and tell Cargo to re-run if the conf changes.
    println!("cargo:rerun-if-changed=config/explorerSettings.conf");
    println!("cargo:rerun-if-changed=build.rs");
}

const DEFAULT_CONF: &str = r#"# ─── Astram Explorer Settings ──────────────────────────────────────────────
# Lines starting with '#' are comments.
# Format: KEY=VALUE

# ── Database ────────────────────────────────────────────────────────────────
# Path to the explorer RocksDB database directory.
DB_PATH=explorer_data

# ── Node connection ─────────────────────────────────────────────────────────
# URL of the Astram node's internal HTTP API.
NODE_RPC_URL=http://127.0.0.1:19533

# ── HTTP server ─────────────────────────────────────────────────────────────
# Address and port the explorer web server listens on.
BIND_ADDR=0.0.0.0
PORT=8080

# ── Sync interval ───────────────────────────────────────────────────────────
# How often (in seconds) to poll the node for new blocks.
SYNC_INTERVAL_SECS=10
"#;
