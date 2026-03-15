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

#[derive(Subcommand, Debug, Clone, PartialEq)]
pub enum RunCmd {
    Simulate {
        /// Configuration toml file for the simulation
        config: PathBuf,
    },
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
    /// Parse and inspect a .nxs trace file
    Parse {
        /// Path to the .nxs trace file
        trace: PathBuf,

        /// Filter by event types (comma-separated: tx,rx,drop,position,energy,motion)
        #[arg(long, value_delimiter = ',')]
        events: Option<Vec<EventFilter>>,

        /// Filter by node names (comma-separated)
        #[arg(long, value_delimiter = ',')]
        nodes: Option<Vec<String>>,

        /// Filter by channel names (comma-separated)
        #[arg(long, value_delimiter = ',')]
        channels: Option<Vec<String>>,

        /// Start timestep (inclusive)
        #[arg(long)]
        from: Option<u64>,

        /// End timestep (inclusive)
        #[arg(long)]
        to: Option<u64>,

        /// Output format
        #[arg(long, default_value = "text")]
        output: ParseOutput,

        /// External adapter command for payload decoding
        #[arg(long)]
        adapter: Option<String>,

        /// Only print the trace header summary
        #[arg(long)]
        header_only: bool,
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
            RunCmd::Simulate { .. } => write!(f, "simulate"),
            RunCmd::Replay { .. } => write!(f, "replay"),
            RunCmd::Logs { .. } => write!(f, "logs"),
            RunCmd::Fuzz => write!(f, "fuzz"),
            RunCmd::Modules { .. } => write!(f, "modules"),
            RunCmd::Parse { .. } => write!(f, "parse"),
        }
    }
}

#[derive(ValueEnum, Debug, Clone, PartialEq)]
pub enum EventFilter {
    Tx,
    Rx,
    Drop,
    Position,
    Energy,
    Motion,
}

#[derive(ValueEnum, Debug, Clone, Default, PartialEq)]
pub enum ParseOutput {
    #[default]
    Text,
    Json,
    JsonLines,
}
