mod cli;
mod governance;

use clap::Parser;
use cli::Cli;

fn main() {
    let cli = Cli::parse();
    println!("Loaded config: {}", cli.config);
}