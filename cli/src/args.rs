use std::{fmt::Display, path::PathBuf, str::FromStr};

use clap::Parser;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Configuration toml file for the simulation
    #[arg(short, long)]
    pub config: String,

    /// Location where the NexusFS should be mounted during simulation
    #[arg(short, long)]
    pub nexus_root: Option<PathBuf>,

    #[arg(short, long, default_value_t = RunMode::Simulate)]
    pub mode: RunMode,
}

#[derive(Debug, Clone, Copy)]
pub enum RunMode {
    Simulate,
    Playback,
}

impl FromStr for RunMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "simulate" => Ok(RunMode::Simulate),
            "playback" => Ok(RunMode::Playback),
            _ => Err(format!("Invalid mode: {}", s)),
        }
    }
}

impl Display for RunMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunMode::Simulate => write!(f, "simulate"),
            RunMode::Playback => write!(f, "playback"),
        }
    }
}
