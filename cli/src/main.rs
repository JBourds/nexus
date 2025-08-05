use anyhow::{Context, Result};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(short, long)]
    /// Configuration toml file for the simulation
    config: String,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let config_text = std::fs::read_to_string(args.config.as_str())
        .context(format!("Unable to open file located at {}", args.config))?;
    let sim = config::parse(config_text)?;
    println!("{sim:#?}");
    Ok(())
}
