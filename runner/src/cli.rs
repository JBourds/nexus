use std::{fmt::Display, path::PathBuf};

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// Command to run
    #[command(subcommand)]
    pub cmd: RunCmd,

    /// How to format the output data.
    #[arg(short, long, default_value_t)]
    pub fmt: OutputFormat,

    /// Which destination to use for summary output
    #[arg(short, long, default_value_t)]
    pub dest: OutputDestination,

    /// Number of times to run unique simulations
    #[arg(long)]
    pub n: Option<usize>,

    /// Configuration toml file for the simulation
    #[arg(short, long)]
    pub config: String,

    /// Location where the NexusFS should be mounted during simulation
    #[arg(short, long)]
    pub root: Option<PathBuf>,
}

#[derive(ValueEnum, Debug, Default, Clone)]
pub enum OutputFormat {
    #[default]
    Csv,
}

impl OutputFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            OutputFormat::Csv => "csv",
        }
    }
}

impl Display for OutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.extension())
    }
}

#[derive(ValueEnum, Debug, Default, Clone)]
pub enum OutputDestination {
    #[default]
    Stdout,
    File,
}

impl Display for OutputDestination {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutputDestination::Stdout => f.write_str("stdout"),
            OutputDestination::File => f.write_str("file"),
        }
    }
}

#[derive(Subcommand, Debug, Default, Clone, PartialEq)]
pub enum RunCmd {
    #[default]
    Simulate,
    Replay {
        logs: PathBuf,
    },
    Logs {
        logs: PathBuf,
    },
    Fuzz,
    /// Manage and inspect reusable module files
    Modules {
        #[command(subcommand)]
        action: ModulesCmd,
    },
}

#[derive(Subcommand, Debug, Clone, PartialEq)]
pub enum ModulesCmd {
    /// List available modules (stdlib + NEXUS_MODULE_PATH)
    List {
        /// Filter by category/directory (e.g. "lora", "boards")
        #[arg(long)]
        category: Option<String>,
    },
    /// Print module contents with descriptions
    Show {
        /// Module specifier (e.g. "lora/sx1276_915mhz")
        module: String,
    },
    /// Verify all `use` imports resolve and no conflicts exist
    Verify {
        /// Path to nexus.toml configuration file
        config: PathBuf,
    },
}

impl Display for RunCmd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunCmd::Simulate => write!(f, "simulate"),
            RunCmd::Replay { .. } => write!(f, "replay"),
            RunCmd::Logs { .. } => write!(f, "logs"),
            RunCmd::Fuzz => write!(f, "fuzz"),
            RunCmd::Modules { .. } => write!(f, "modules"),
        }
    }
}
