use chrono::{DateTime, Utc};
use fuse::channel::{ChannelMode, NexusChannel};
use kernel::{self, Kernel, sources::Source};
use libc::{O_RDONLY, O_RDWR, O_WRONLY};
use runner::cli::OutputDestination;
use runner::{ProtocolHandle, ProtocolSummary};
use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::stdout;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use tracing_subscriber::{EnvFilter, filter, fmt, prelude::*};

use anyhow::Result;
use clap::Parser;
use config::ast::{self, ChannelType};
use fuse::{PID, fs::*};

use runner::{cli::Cli, cli::ModulesCmd, cli::RunCmd};
use std::path::PathBuf;

use crate::output::to_csv;

mod output;

const CONFIG: &str = "nexus.toml";

fn main() -> Result<()> {
    let args = Cli::parse();
    match &args.cmd {
        RunCmd::Simulate { .. } => simulate(args),
        RunCmd::Replay { .. } => replay(args),
        RunCmd::Logs { .. } => print_logs(args),
        RunCmd::Modules { action } => handle_modules(action),
        _ => todo!(),
    }
}

fn simulate(args: Cli) -> Result<()> {
    let RunCmd::Simulate { ref config } = args.cmd else {
        unreachable!()
    };
    let sim = config::parse(config.clone())?;
    let root = make_sim_dir(&sim.params.root)?;
    config::serialize_config(&sim, &root.join(CONFIG))?;
    run(args, sim, root)
}

fn replay(args: Cli) -> Result<()> {
    let RunCmd::Replay { logs } = &args.cmd else {
        unreachable!()
    };
    let sim = config::deserialize_config(&logs.join(CONFIG))?;
    let root = make_sim_dir(&sim.params.root)?;
    config::serialize_config(&sim, &root.join(CONFIG))?;
    run(args, sim, root)
}

fn print_logs(args: Cli) -> Result<()> {
    let RunCmd::Logs { logs } = &args.cmd else {
        unreachable!()
    };
    Source::print_logs(logs)?;
    Ok(())
}

fn handle_modules(action: &ModulesCmd) -> Result<()> {
    match action {
        ModulesCmd::List { category } => {
            let stdlib = config::module::stdlib_path();
            let mut dirs: Vec<PathBuf> = vec![];

            // Collect search directories: NEXUS_MODULE_PATH + stdlib.
            if let Ok(search_path) = std::env::var("NEXUS_MODULE_PATH") {
                for dir in search_path.split(':') {
                    let p = PathBuf::from(dir);
                    if p.is_dir() {
                        dirs.push(p);
                    }
                }
            }
            if stdlib.is_dir() {
                dirs.push(stdlib.to_path_buf());
            }

            if dirs.is_empty() {
                println!("No module directories found.");
                return Ok(());
            }

            for dir in &dirs {
                println!("# {}", dir.display());
                list_modules(dir, category.as_deref(), "")?;
                println!();
            }
            Ok(())
        }
        ModulesCmd::Show { module } => {
            let path = config::module::resolve_module_path(module, None)?;
            let contents = std::fs::read_to_string(&path)?;
            println!("# {}\n", path.display());
            print!("{contents}");
            Ok(())
        }
        ModulesCmd::Verify { config } => {
            match config::parse(config.clone()) {
                Ok(_) => println!("OK: all modules resolved, no conflicts."),
                Err(e) => println!("ERROR: {e:#}"),
            }
            Ok(())
        }
    }
}

fn list_modules(
    dir: &Path,
    category: Option<&str>,
    prefix: &str,
) -> Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let ft = entry.file_type()?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if ft.is_dir() {
            let sub_prefix = if prefix.is_empty() {
                name_str.to_string()
            } else {
                format!("{prefix}/{name_str}")
            };
            // If category filter is set, only recurse into matching dirs.
            if let Some(cat) = category {
                if !name_str.eq_ignore_ascii_case(cat) && !sub_prefix.starts_with(cat) {
                    continue;
                }
            }
            list_modules(&entry.path(), category, &sub_prefix)?;
        } else if ft.is_file() && name_str.ends_with(".toml") {
            let stem = name_str.trim_end_matches(".toml");
            let spec = if prefix.is_empty() {
                stem.to_string()
            } else {
                format!("{prefix}/{stem}")
            };
            println!("  {spec}");
        }
    }
    Ok(())
}

fn run(args: Cli, sim: ast::Simulation, root: PathBuf) -> Result<()> {
    ctrlc::set_handler(|| {}).expect("Error setting signal termination handler");

    println!("Simulation Root: {}", root.to_string_lossy());
    #[allow(unused_variables)]
    let trace_path = setup_logging(root.as_path(), &args.cmd, &sim)?;
    runner::build(&sim)?;
    let mut summaries = vec![];
    for _ in 0..args.n.unwrap_or(1) {
        let runc = runner::run(&sim)?;
        let protocol_channels = make_fs_channels(&sim, &runc.handles, &args.cmd)?;
        let pending_remaps = Arc::new(Mutex::new(Vec::new()));
        let fs = args
            .root
            .clone()
            .map(|root| NexusFs::new(root, pending_remaps.clone()))
            .unwrap_or_default();

        #[allow(unused_variables)]
        let (sess, (tx, rx)) = fs
            .add_processes(&runc.handles)
            .add_channels(protocol_channels)?
            .mount()
            .expect("unable to mount file system");

        // Need to join fs thread so the other processes don't get stuck
        // in an uninterruptible sleep state.
        let file_handles = make_file_handles(&sim, &runc.handles);
        let protocol_handles = Kernel::new(
            sim.clone(),
            runc,
            file_handles,
            rx,
            tx,
            pending_remaps.clone(),
        )?
        .run(args.cmd.clone())?;
        summaries.extend(get_output(protocol_handles));
    }
    match args.dest {
        OutputDestination::Stdout => {
            to_csv(stdout(), &summaries);
        }
        OutputDestination::File => {
            let path = root.join(format!("output.{}", args.fmt.extension()));
            let f = OpenOptions::new().write(true).create_new(true).open(path)?;
            to_csv(f, &summaries);
        }
    }
    Ok(())
}

#[allow(dead_code)]
fn fuzz(_args: Cli) -> Result<()> {
    todo!()
}

fn get_output(handles: Vec<ProtocolHandle>) -> Vec<ProtocolSummary> {
    handles
        .into_iter()
        .filter_map(ProtocolHandle::finish)
        .collect()
}

fn make_sim_dir(sim_root: &Path) -> Result<PathBuf> {
    let datetime: DateTime<Utc> = SystemTime::now().into();
    let datetime = datetime.format("%Y-%m-%d_%H:%M:%S").to_string();
    let root = sim_root.join(&datetime);
    if !root.exists() {
        std::fs::create_dir_all(&root)?;
    }
    Ok(root)
}

fn setup_logging(root: &Path, cmd: &RunCmd, sim: &ast::Simulation) -> Result<PathBuf> {
    let trace_path = root.join("trace.nxs");

    // Build TraceLayer for binary logging (both tx and rx go into unified trace)
    let trace_layer = if matches!(cmd, RunCmd::Simulate { .. } | RunCmd::Replay { .. }) {
        let header = trace::format::TraceHeader {
            node_names: {
                let mut names: Vec<_> = sim.nodes.keys().cloned().collect();
                names.sort();
                names
            },
            channel_names: {
                let mut names: Vec<_> = sim.channels.keys().cloned().collect();
                names.sort();
                names
            },
            timestep_count: sim.params.timestep.count.get(),
            node_max_nj: {
                let mut names: Vec<_> = sim.nodes.keys().cloned().collect();
                names.sort();
                names
                    .iter()
                    .map(|n| sim.nodes[n].charge.as_ref().map(|c| c.unit.to_nj(c.max)))
                    .collect()
            },
        };
        Some(trace::layer::TraceLayer::new(&trace_path, &header)?)
    } else {
        None
    };

    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_filter(filter::filter_fn(|metadata| {
                    !matches!(
                        metadata.target(),
                        "tx" | "rx" | "drop" | "battery" | "movement" | "motion"
                    )
                }))
                .with_filter(EnvFilter::from_default_env()),
        )
        .with(trace_layer.map(|layer| {
            layer.with_filter(filter::filter_fn(|metadata| {
                matches!(
                    metadata.target(),
                    "tx" | "rx" | "drop" | "battery" | "movement" | "motion"
                )
            }))
        }))
        .init();
    Ok(trace_path)
}

fn make_file_handles(
    sim: &ast::Simulation,
    handles: &[runner::ProtocolHandle],
) -> Vec<(PID, ast::NodeHandle, ast::ChannelHandle)> {
    let mut res = vec![];
    for runner::ProtocolHandle {
        node: node_handle,
        protocol: protocol_handle,
        process,
        ..
    } in handles
    {
        let node = &sim.nodes.get(node_handle).unwrap();
        let protocol = node.protocols.get(protocol_handle).unwrap();
        let pid = process.as_ref().unwrap().id();

        for channel in protocol
            .subscribers
            .iter()
            .chain(protocol.publishers.iter())
            .collect::<HashSet<&ast::ChannelHandle>>()
            .into_iter()
            // also add control files in here because it makes resolution easier
            .chain(control_files().iter())
        {
            res.push((pid, node_handle.clone(), channel.clone()));
        }
    }
    res
}

fn make_fs_channels(
    sim: &ast::Simulation,
    handles: &[runner::ProtocolHandle],
    run_cmd: &RunCmd,
) -> Result<Vec<NexusChannel>, fuse::errors::ChannelError> {
    let mut channels = vec![];
    for runner::ProtocolHandle {
        node: node_handle,
        protocol: protocol_handle,
        process,
        ..
    } in handles
    {
        let node = &sim.nodes.get(node_handle).unwrap();
        let protocol = node.protocols.get(protocol_handle).unwrap();
        let pid = process.as_ref().unwrap().id();

        for channel in protocol
            .subscribers
            .iter()
            .chain(protocol.publishers.iter())
            .collect::<HashSet<&ast::ChannelHandle>>()
            .into_iter()
        {
            let mode = match run_cmd {
                RunCmd::Simulate { .. } => {
                    let file_cmd = match (
                        protocol.subscribers.contains(channel),
                        protocol.publishers.contains(channel),
                    ) {
                        (true, true) => O_RDWR,
                        (true, _) => O_RDONLY,
                        (_, true) => O_WRONLY,
                        _ => unreachable!(),
                    };
                    ChannelMode::try_from(file_cmd)?
                }
                RunCmd::Replay { .. } => ChannelMode::ReplayWrites,
                RunCmd::Fuzz => ChannelMode::FuzzWrites,
                _ => unreachable!(),
            };

            channels.push(NexusChannel {
                pid,
                node: node_handle.clone(),
                channel: channel.clone(),
                mode,
                max_msg_size: sim
                    .channels
                    .get(channel)
                    .map(|ch| ch.r#type.max_buf_size())
                    .unwrap_or(ChannelType::MSG_MAX_DEFAULT),
            });
        }
    }
    Ok(channels)
}
