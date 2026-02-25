mod cli;
#[allow(dead_code)]
mod controller;
mod crd;
#[allow(dead_code)]
mod governance;
mod reconcile;

use clap::Parser;
use cli::Cli;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    println!("Loaded config: {}", cli.config);
    println!("Async runtime initialized");
    Ok(())
}
