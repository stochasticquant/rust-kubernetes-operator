use clap::Parser;

#[derive(Parser, Debug)]
pub struct Cli {
    #[arg(long)]
    pub config: String,
}
