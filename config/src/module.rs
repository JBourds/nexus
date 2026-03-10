use std::collections::HashMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use tracing::warn;

use crate::parse;

const STDLIB_DIR: &str = env!("NEXUS_STDLIB_DIR");

/// Origin of a definition for conflict error messages.
#[derive(Debug, Clone)]
struct Origin {
    path: PathBuf,
}

/// Accumulated module definitions before merge into the main simulation.
#[derive(Debug, Default)]
struct ResolvedModules {
    links: HashMap<String, (parse::Link, Origin)>,
    channels: HashMap<String, (parse::Channel, Origin)>,
    profiles: HashMap<String, (parse::NodeProfile, Origin)>,
}

/// Resolve all modules referenced by `use_list` and merge them into `sim`.
pub(crate) fn resolve_and_merge(
    config_dir: &Path,
    sim: &mut parse::Simulation,
) -> Result<()> {
    let use_list = match sim.r#use.take() {
        Some(list) if !list.is_empty() => list,
        _ => return Ok(()),
    };

    let mut visited = HashSet::new();
    let mut stack = HashSet::new();
    let mut result = ResolvedModules::default();

    for spec in &use_list {
        resolve_recursive(config_dir, spec, &mut visited, &mut stack, &mut result)
            .with_context(|| format!("Failed to resolve module \"{spec}\""))?;
    }

    merge_into_simulation(sim, result)
}

/// Resolve a single module spec and recurse into its transitive dependencies.
fn resolve_recursive(
    base_dir: &Path,
    spec: &str,
    visited: &mut HashSet<PathBuf>,
    stack: &mut HashSet<PathBuf>,
    result: &mut ResolvedModules,
) -> Result<()> {
    let path = resolve_path(base_dir, spec)
        .with_context(|| format!("Could not resolve module path for \"{spec}\""))?;

    // Already loaded — skip.
    if visited.contains(&path) {
        return Ok(());
    }

    // Cycle detection.
    if stack.contains(&path) {
        bail!("Circular module import detected: \"{}\"", path.display());
    }
    stack.insert(path.clone());

    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("Unable to read module file at \"{}\"", path.display()))?;
    let module: parse::ModuleFile = toml::from_str(&text)
        .with_context(|| format!("Failed to parse module file at \"{}\"", path.display()))?;

    // Recurse into transitive dependencies, resolving relative to this module's dir.
    let module_dir = path.parent().unwrap_or(base_dir);
    if let Some(ref uses) = module.r#use {
        for child_spec in uses {
            resolve_recursive(module_dir, child_spec, visited, stack, result)
                .with_context(|| {
                    format!(
                        "While loading transitive dependency \"{child_spec}\" from \"{}\"",
                        path.display()
                    )
                })?;
        }
    }

    let origin = Origin { path: path.clone() };

    // Merge links.
    for (name, link) in module.links {
        let key = name.to_ascii_lowercase();
        if let Some((_, existing)) = result.links.get(&key) {
            bail!(
                "Duplicate link \"{}\" defined in both \"{}\" and \"{}\"",
                name,
                existing.path.display(),
                origin.path.display()
            );
        }
        result.links.insert(key, (link, origin.clone()));
    }

    // Merge channels.
    for (name, channel) in module.channels {
        let key = name.to_ascii_lowercase();
        if let Some((_, existing)) = result.channels.get(&key) {
            bail!(
                "Duplicate channel \"{}\" defined in both \"{}\" and \"{}\"",
                name,
                existing.path.display(),
                origin.path.display()
            );
        }
        result.channels.insert(key, (channel, origin.clone()));
    }

    // Merge profiles.
    if let Some(profiles) = module.profiles {
        for (name, profile) in profiles {
            let key = name.to_ascii_lowercase();
            if let Some((_, existing)) = result.profiles.get(&key) {
                bail!(
                    "Duplicate profile \"{}\" defined in both \"{}\" and \"{}\"",
                    name,
                    existing.path.display(),
                    origin.path.display()
                );
            }
            result.profiles.insert(key, (profile, origin.clone()));
        }
    }

    stack.remove(&path);
    visited.insert(path);
    Ok(())
}

/// Resolve a module specifier to a canonical filesystem path.
fn resolve_path(base_dir: &Path, spec: &str) -> Result<PathBuf> {
    let with_ext = |p: PathBuf| -> PathBuf {
        if p.extension().is_some() {
            p
        } else {
            p.with_extension("toml")
        }
    };

    // Relative or absolute path.
    if spec.starts_with("./") || spec.starts_with("../") || spec.starts_with('/') {
        let candidate = with_ext(base_dir.join(spec));
        if candidate.exists() {
            return std::fs::canonicalize(&candidate).with_context(|| {
                format!("Failed to canonicalize path \"{}\"", candidate.display())
            });
        }
        bail!("Module file not found at \"{}\"", candidate.display());
    }

    // Search NEXUS_MODULE_PATH (colon-separated).
    if let Ok(search_path) = std::env::var("NEXUS_MODULE_PATH") {
        for dir in search_path.split(':') {
            let dir = Path::new(dir);
            let candidate = with_ext(dir.join(spec));
            if candidate.exists() {
                return std::fs::canonicalize(&candidate).with_context(|| {
                    format!("Failed to canonicalize path \"{}\"", candidate.display())
                });
            }
        }
    }

    // Standard library.
    let stdlib = Path::new(STDLIB_DIR);
    let candidate = with_ext(stdlib.join(spec));
    if candidate.exists() {
        return std::fs::canonicalize(&candidate)
            .with_context(|| format!("Failed to canonicalize path \"{}\"", candidate.display()));
    }

    bail!(
        "Module \"{spec}\" not found in NEXUS_MODULE_PATH or standard library ({})",
        STDLIB_DIR
    );
}

/// Merge resolved module definitions into the parse-level simulation.
/// User definitions take precedence (with a warning).
fn merge_into_simulation(
    sim: &mut parse::Simulation,
    modules: ResolvedModules,
) -> Result<()> {
    for (name, (link, _origin)) in modules.links {
        if sim.links.contains_key(&name) {
            warn!("Link \"{name}\" in nexus.toml overrides module definition");
        } else {
            sim.links.insert(name, link);
        }
    }

    for (name, (channel, _origin)) in modules.channels {
        if sim.channels.contains_key(&name) {
            warn!("Channel \"{name}\" in nexus.toml overrides module definition");
        } else {
            sim.channels.insert(name, channel);
        }
    }

    let profiles = sim.profiles.get_or_insert_with(HashMap::new);
    for (name, (profile, _origin)) in modules.profiles {
        if profiles.contains_key(&name) {
            warn!("Profile \"{name}\" in nexus.toml overrides module definition");
        } else {
            profiles.insert(name, profile);
        }
    }

    Ok(())
}

/// Apply a profile's fields to a node (profile defaults, user overrides).
pub(crate) fn apply_profile(node: &mut parse::Node, profile: &parse::NodeProfile) {
    // Resources: per-field fallback to profile.
    match (&mut node.resources, &profile.resources) {
        (None, Some(_)) => {
            node.resources = profile.resources.clone();
        }
        (Some(nr), Some(pr)) => {
            if nr.clock_rate.is_none() {
                nr.clock_rate = pr.clock_rate;
            }
            if nr.cores.is_none() {
                nr.cores = pr.cores;
            }
            if nr.clock_units.is_none() {
                nr.clock_units = pr.clock_units.clone();
            }
            if nr.ram.is_none() {
                nr.ram = pr.ram;
            }
            if nr.ram_units.is_none() {
                nr.ram_units = pr.ram_units.clone();
            }
        }
        _ => {}
    }

    // Maps: union with user-wins.
    merge_option_map(&mut node.power_states, &profile.power_states);
    merge_option_map(&mut node.power_sources, &profile.power_sources);
    merge_option_map(&mut node.power_sinks, &profile.power_sinks);
    merge_option_map(&mut node.channel_energy, &profile.channel_energy);
}

fn merge_option_map<V: Clone>(
    target: &mut Option<HashMap<String, V>>,
    source: &Option<HashMap<String, V>>,
) {
    match (target.as_mut(), source) {
        (Some(t), Some(s)) => {
            for (k, v) in s {
                t.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
        (None, Some(s)) => {
            *target = Some(s.clone());
        }
        _ => {}
    }
}

// --- Public helpers for CLI ---

/// The standard library modules directory.
pub fn stdlib_path() -> &'static Path {
    Path::new(STDLIB_DIR)
}

/// Resolve a module specifier to a filesystem path (for `nexus modules show`).
pub fn resolve_module_path(spec: &str, config_dir: Option<&Path>) -> Result<PathBuf> {
    let base = config_dir.unwrap_or_else(|| Path::new("."));
    resolve_path(base, spec)
}
