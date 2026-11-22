//! AO3Instance server: serves the reader UI over HTTP.
//!
//! `cargo run --bin ao3i-server` (or `nix run .`) and open the printed URL.

use ao3instance::config::Config;
use ao3instance::server;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_env()?;
    println!("data dir: {}", config.data_dir.display());
    println!("database: {}", config.db_file.display());
    server::run(&config).await?;
    Ok(())
}
