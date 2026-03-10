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
/// Keys are lowercased for case-insensitive duplicate detection.
/// The `original_name` field preserves the author's original casing.
#[derive(Debug, Default)]
struct ResolvedModules {
    links: HashMap<String, (parse::Link, Origin, String)>,
    channels: HashMap<String, (parse::Channel, Origin, String)>,
    profiles: HashMap<String, (parse::NodeProfile, Origin, String)>,
}

/// Resolve all modules referenced by `use_list` and merge them into `sim`.
pub(crate) fn resolve_and_merge(config_dir: &Path, sim: &mut parse::Simulation) -> Result<()> {
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
            resolve_recursive(module_dir, child_spec, visited, stack, result).with_context(
                || {
                    format!(
                        "While loading transitive dependency \"{child_spec}\" from \"{}\"",
                        path.display()
                    )
                },
            )?;
        }
    }

    let origin = Origin { path: path.clone() };

    // Merge links.
    for (name, link) in module.links {
        let key = name.to_ascii_lowercase();
        if let Some((_, existing, _)) = result.links.get(&key) {
            bail!(
                "Duplicate link \"{}\" defined in both \"{}\" and \"{}\"",
                name,
                existing.path.display(),
                origin.path.display()
            );
        }
        result.links.insert(key, (link, origin.clone(), name));
    }

    // Merge channels.
    for (name, channel) in module.channels {
        let key = name.to_ascii_lowercase();
        if let Some((_, existing, _)) = result.channels.get(&key) {
            bail!(
                "Duplicate channel \"{}\" defined in both \"{}\" and \"{}\"",
                name,
                existing.path.display(),
                origin.path.display()
            );
        }
        result.channels.insert(key, (channel, origin.clone(), name));
    }

    // Merge profiles.
    if let Some(profiles) = module.profiles {
        for (name, profile) in profiles {
            let key = name.to_ascii_lowercase();
            if let Some((_, existing, _)) = result.profiles.get(&key) {
                bail!(
                    "Duplicate profile \"{}\" defined in both \"{}\" and \"{}\"",
                    name,
                    existing.path.display(),
                    origin.path.display()
                );
            }
            result.profiles.insert(key, (profile, origin.clone(), name));
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
/// Comparison is case-insensitive: a user-defined `[links.LoRa]` overrides
/// a module-defined `[links.lora]`.
fn merge_into_simulation(sim: &mut parse::Simulation, modules: ResolvedModules) -> Result<()> {
    // Build a set of lowercased user-defined keys for case-insensitive override detection.
    let user_link_keys: HashSet<String> = sim.links.keys().map(|k| k.to_ascii_lowercase()).collect();
    for (_key, (link, _origin, original_name)) in modules.links {
        if user_link_keys.contains(&original_name.to_ascii_lowercase()) {
            warn!(
                "Link \"{original_name}\" in nexus.toml overrides module definition",
            );
        } else {
            sim.links.insert(original_name, link);
        }
    }

    let user_channel_keys: HashSet<String> = sim.channels.keys().map(|k| k.to_ascii_lowercase()).collect();
    for (_key, (channel, _origin, original_name)) in modules.channels {
        if user_channel_keys.contains(&original_name.to_ascii_lowercase()) {
            warn!(
                "Channel \"{original_name}\" in nexus.toml overrides module definition",
            );
        } else {
            sim.channels.insert(original_name, channel);
        }
    }

    let profiles = sim.profiles.get_or_insert_with(HashMap::new);
    let user_profile_keys: HashSet<String> = profiles.keys().map(|k| k.to_ascii_lowercase()).collect();
    for (_key, (profile, _origin, original_name)) in modules.profiles {
        if user_profile_keys.contains(&original_name.to_ascii_lowercase()) {
            warn!(
                "Profile \"{original_name}\" in nexus.toml overrides module definition",
            );
        } else {
            // Store under the lowercased name so profile lookups are case-insensitive.
            profiles.insert(original_name.to_ascii_lowercase(), profile);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::num::NonZeroU64;

    /// Create a temp dir with module files for testing.
    fn setup_temp_modules(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, content) in files {
            let path = dir.path().join(name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, content).unwrap();
        }
        dir
    }

    #[test]
    fn resolve_stdlib_module() {
        let path = resolve_path(Path::new("/tmp"), "lora/sx1276_915mhz").unwrap();
        assert!(path.exists());
        assert!(path.to_string_lossy().ends_with("sx1276_915mhz.toml"));
    }

    #[test]
    fn resolve_relative_module() {
        let dir = setup_temp_modules(&[("my_module.toml", "[links]\n")]);
        let path = resolve_path(dir.path(), "./my_module").unwrap();
        assert!(path.exists());
    }

    #[test]
    fn resolve_not_found() {
        let err = resolve_path(Path::new("/tmp"), "nonexistent/module").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("not found"), "unexpected error: {msg}");
    }

    #[test]
    fn resolve_appends_toml_extension() {
        let path = resolve_path(Path::new("/tmp"), "boards/esp32_devkit").unwrap();
        assert!(path.to_string_lossy().ends_with(".toml"));
    }

    #[test]
    fn module_with_params_rejected() {
        let dir = setup_temp_modules(&[("bad.toml", "[params]\nseed = 42\n")]);
        let mut sim = parse::Simulation::default();
        sim.r#use = Some(vec!["./bad".to_string()]);
        let err = resolve_and_merge(dir.path(), &mut sim).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("unknown field `params`"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn module_with_nodes_rejected() {
        let dir = setup_temp_modules(&[("bad.toml", "[nodes.x]\ndeployments = [{}]\n")]);
        let mut sim = parse::Simulation::default();
        sim.r#use = Some(vec!["./bad".to_string()]);
        let err = resolve_and_merge(dir.path(), &mut sim).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("unknown field `nodes`"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn circular_import_detected() {
        let dir = setup_temp_modules(&[
            ("a.toml", "use = [\"./b\"]\n"),
            ("b.toml", "use = [\"./a\"]\n"),
        ]);
        let mut sim = parse::Simulation::default();
        sim.r#use = Some(vec!["./a".to_string()]);
        let err = resolve_and_merge(dir.path(), &mut sim).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("Circular module import"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn transitive_import() {
        let dir = setup_temp_modules(&[
            (
                "a.toml",
                "use = [\"./b\"]\n\n[links.link_a]\nmedium.type = \"wireless\"\n\
                 medium.wavelength_meters = 0.3\nmedium.gain_dbi = 2.0\n\
                 medium.rx_min_dbm = -100.0\nmedium.tx_min_dbm = -4.0\n\
                 medium.tx_max_dbm = 20.0\n",
            ),
            (
                "b.toml",
                "[links.link_b]\nmedium.type = \"wireless\"\n\
                 medium.wavelength_meters = 0.5\nmedium.gain_dbi = 2.0\n\
                 medium.rx_min_dbm = -100.0\nmedium.tx_min_dbm = -4.0\n\
                 medium.tx_max_dbm = 20.0\n",
            ),
        ]);
        let mut sim = parse::Simulation::default();
        sim.r#use = Some(vec!["./a".to_string()]);
        resolve_and_merge(dir.path(), &mut sim).unwrap();
        assert!(sim.links.contains_key("link_a"));
        assert!(sim.links.contains_key("link_b"));
    }

    #[test]
    fn duplicate_link_across_modules_rejected() {
        let dir = setup_temp_modules(&[
            (
                "a.toml",
                "[links.shared_name]\nmedium.type = \"wireless\"\n\
                 medium.wavelength_meters = 0.3\nmedium.gain_dbi = 2.0\n\
                 medium.rx_min_dbm = -100.0\nmedium.tx_min_dbm = -4.0\n\
                 medium.tx_max_dbm = 20.0\n",
            ),
            (
                "b.toml",
                "[links.shared_name]\nmedium.type = \"wireless\"\n\
                 medium.wavelength_meters = 0.5\nmedium.gain_dbi = 2.0\n\
                 medium.rx_min_dbm = -100.0\nmedium.tx_min_dbm = -4.0\n\
                 medium.tx_max_dbm = 20.0\n",
            ),
        ]);
        let mut sim = parse::Simulation::default();
        sim.r#use = Some(vec!["./a".to_string(), "./b".to_string()]);
        let err = resolve_and_merge(dir.path(), &mut sim).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("Duplicate link"), "unexpected error: {msg}");
    }

    #[test]
    fn user_definition_overrides_module() {
        let dir = setup_temp_modules(&[(
            "m.toml",
            "[links.mylink]\nmedium.type = \"wireless\"\n\
             medium.wavelength_meters = 0.3\nmedium.gain_dbi = 2.0\n\
             medium.rx_min_dbm = -100.0\nmedium.tx_min_dbm = -4.0\n\
             medium.tx_max_dbm = 20.0\n",
        )]);
        let mut sim = parse::Simulation::default();
        sim.r#use = Some(vec!["./m".to_string()]);
        // User also defines "mylink".
        sim.links
            .insert("mylink".to_string(), parse::Link::default());
        resolve_and_merge(dir.path(), &mut sim).unwrap();
        // User's link should remain (it's the Default, not the module's wireless one).
        assert!(sim.links["mylink"].medium.is_none());
    }

    #[test]
    fn deduplication_by_path() {
        // If same module is imported twice (directly and transitively), load only once.
        let dir = setup_temp_modules(&[
            (
                "a.toml",
                "use = [\"./common\"]\n\n[links.link_a]\nmedium.type = \"wireless\"\n\
                 medium.wavelength_meters = 0.3\nmedium.gain_dbi = 2.0\n\
                 medium.rx_min_dbm = -100.0\nmedium.tx_min_dbm = -4.0\n\
                 medium.tx_max_dbm = 20.0\n",
            ),
            (
                "common.toml",
                "[links.shared]\nmedium.type = \"wireless\"\n\
                 medium.wavelength_meters = 0.5\nmedium.gain_dbi = 2.0\n\
                 medium.rx_min_dbm = -100.0\nmedium.tx_min_dbm = -4.0\n\
                 medium.tx_max_dbm = 20.0\n",
            ),
        ]);
        let mut sim = parse::Simulation::default();
        // Both ./a (which uses ./common) and ./common directly.
        sim.r#use = Some(vec!["./a".to_string(), "./common".to_string()]);
        resolve_and_merge(dir.path(), &mut sim).unwrap();
        assert!(sim.links.contains_key("link_a"));
        assert!(sim.links.contains_key("shared"));
    }

    #[test]
    fn profile_apply_resources() {
        let profile = parse::NodeProfile {
            resources: Some(parse::Resources {
                clock_rate: NonZeroU64::new(240),
                clock_units: Some(parse::Unit("mhz".to_string())),
                ram: NonZeroU64::new(512),
                ram_units: Some(parse::Unit("kb".to_string())),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut node = parse::Node::default();
        apply_profile(&mut node, &profile);
        let res = node.resources.unwrap();
        assert_eq!(res.clock_rate, NonZeroU64::new(240));
        assert_eq!(res.ram, NonZeroU64::new(512));
    }

    #[test]
    fn profile_resources_user_wins() {
        let profile = parse::NodeProfile {
            resources: Some(parse::Resources {
                clock_rate: NonZeroU64::new(240),
                clock_units: Some(parse::Unit("mhz".to_string())),
                ram: NonZeroU64::new(512),
                ram_units: Some(parse::Unit("kb".to_string())),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut node = parse::Node {
            resources: Some(parse::Resources {
                clock_rate: NonZeroU64::new(160),
                clock_units: Some(parse::Unit("mhz".to_string())),
                ..Default::default()
            }),
            ..Default::default()
        };
        apply_profile(&mut node, &profile);
        let res = node.resources.unwrap();
        // User's clock_rate wins.
        assert_eq!(res.clock_rate, NonZeroU64::new(160));
        // Profile's ram fills in.
        assert_eq!(res.ram, NonZeroU64::new(512));
    }

    #[test]
    fn profile_map_merge() {
        let mut profile_states = HashMap::new();
        profile_states.insert(
            "active".to_string(),
            parse::PowerRate {
                rate: 100,
                unit: parse::Unit("mw".to_string()),
                time: parse::Unit("s".to_string()),
            },
        );
        profile_states.insert(
            "sleep".to_string(),
            parse::PowerRate {
                rate: 10,
                unit: parse::Unit("uw".to_string()),
                time: parse::Unit("s".to_string()),
            },
        );
        let profile = parse::NodeProfile {
            power_states: Some(profile_states),
            ..Default::default()
        };

        let mut user_states = HashMap::new();
        user_states.insert(
            "active".to_string(),
            parse::PowerRate {
                rate: 200,
                unit: parse::Unit("mw".to_string()),
                time: parse::Unit("s".to_string()),
            },
        );
        let mut node = parse::Node {
            power_states: Some(user_states),
            ..Default::default()
        };
        apply_profile(&mut node, &profile);

        let states = node.power_states.unwrap();
        // User's "active" wins.
        assert_eq!(states["active"].rate, 200);
        // Profile's "sleep" is added.
        assert_eq!(states["sleep"].rate, 10);
    }

    #[test]
    fn profile_no_overrides() {
        let mut profile_sinks = HashMap::new();
        profile_sinks.insert(
            "mcu".to_string(),
            parse::PowerFlowDef::Constant {
                rate: 30,
                unit: parse::Unit("mw".to_string()),
                time: parse::Unit("s".to_string()),
            },
        );
        let profile = parse::NodeProfile {
            power_sinks: Some(profile_sinks),
            ..Default::default()
        };
        let mut node = parse::Node::default();
        apply_profile(&mut node, &profile);
        assert!(node.power_sinks.is_some());
        assert!(node.power_sinks.unwrap().contains_key("mcu"));
    }

    #[test]
    fn nexus_module_path_env() {
        let dir = setup_temp_modules(&[(
            "custom/my_mod.toml",
            "[links.custom_link]\nmedium.type = \"wireless\"\n\
             medium.wavelength_meters = 0.3\nmedium.gain_dbi = 2.0\n\
             medium.rx_min_dbm = -100.0\nmedium.tx_min_dbm = -4.0\n\
             medium.tx_max_dbm = 20.0\n",
        )]);

        // Temporarily set NEXUS_MODULE_PATH.
        unsafe { std::env::set_var("NEXUS_MODULE_PATH", dir.path().to_str().unwrap()) };

        let path = resolve_path(Path::new("/tmp"), "custom/my_mod");
        // Clean up env before asserting.
        unsafe { std::env::remove_var("NEXUS_MODULE_PATH") };

        let path = path.unwrap();
        assert!(path.exists());
        assert!(path.to_string_lossy().contains("my_mod.toml"));
    }

    #[test]
    fn stdlib_modules_all_parse() {
        // Verify every .toml file in the stdlib parses as a valid ModuleFile.
        let stdlib = Path::new(STDLIB_DIR);
        if !stdlib.is_dir() {
            return; // skip if not present (e.g., CI without modules/)
        }
        fn walk(dir: &Path) {
            for entry in fs::read_dir(dir).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                if path.is_dir() {
                    walk(&path);
                } else if path.extension().is_some_and(|ext| ext == "toml") {
                    let text = fs::read_to_string(&path).unwrap();
                    let result: Result<parse::ModuleFile, _> = toml::from_str(&text);
                    assert!(
                        result.is_ok(),
                        "Failed to parse stdlib module at {}: {:?}",
                        path.display(),
                        result.err()
                    );
                }
            }
        }
        walk(stdlib);
    }

    #[test]
    fn multi_profile_first_wins_resources() {
        // When two profiles both set the same resource field, the first
        // profile applied wins (it's already set, so later ones don't
        // override).
        let p1 = parse::NodeProfile {
            resources: Some(parse::Resources {
                clock_rate: NonZeroU64::new(240),
                clock_units: Some(parse::Unit("mhz".to_string())),
                ..Default::default()
            }),
            ..Default::default()
        };
        let p2 = parse::NodeProfile {
            resources: Some(parse::Resources {
                clock_rate: NonZeroU64::new(80),
                clock_units: Some(parse::Unit("mhz".to_string())),
                ram: NonZeroU64::new(256),
                ram_units: Some(parse::Unit("kb".to_string())),
                ..Default::default()
            }),
            ..Default::default()
        };

        let mut node = parse::Node::default();
        apply_profile(&mut node, &p1);
        apply_profile(&mut node, &p2);

        let res = node.resources.unwrap();
        // p1's clock_rate wins over p2's.
        assert_eq!(res.clock_rate, NonZeroU64::new(240));
        // p2's ram fills the gap left by p1.
        assert_eq!(res.ram, NonZeroU64::new(256));
    }

    #[test]
    fn multi_profile_first_wins_map_keys() {
        // When two profiles define the same map key, the first profile's
        // value wins because merge uses `or_insert`.
        let p1 = parse::NodeProfile {
            power_states: Some(HashMap::from([(
                "active".to_string(),
                parse::PowerRate {
                    rate: 100,
                    unit: parse::Unit("mw".to_string()),
                    time: parse::Unit("s".to_string()),
                },
            )])),
            ..Default::default()
        };
        let p2 = parse::NodeProfile {
            power_states: Some(HashMap::from([
                (
                    "active".to_string(),
                    parse::PowerRate {
                        rate: 999,
                        unit: parse::Unit("mw".to_string()),
                        time: parse::Unit("s".to_string()),
                    },
                ),
                (
                    "deep_sleep".to_string(),
                    parse::PowerRate {
                        rate: 1,
                        unit: parse::Unit("uw".to_string()),
                        time: parse::Unit("s".to_string()),
                    },
                ),
            ])),
            ..Default::default()
        };

        let mut node = parse::Node::default();
        apply_profile(&mut node, &p1);
        apply_profile(&mut node, &p2);

        let states = node.power_states.unwrap();
        // p1's "active" value wins.
        assert_eq!(states["active"].rate, 100);
        // p2's "deep_sleep" fills in.
        assert_eq!(states["deep_sleep"].rate, 1);
    }

    #[test]
    fn multi_profile_user_overrides_all() {
        // User-defined fields win over all profiles, regardless of order.
        let p1 = parse::NodeProfile {
            resources: Some(parse::Resources {
                clock_rate: NonZeroU64::new(240),
                clock_units: Some(parse::Unit("mhz".to_string())),
                ram: NonZeroU64::new(520),
                ram_units: Some(parse::Unit("kb".to_string())),
                ..Default::default()
            }),
            power_sinks: Some(HashMap::from([(
                "mcu".to_string(),
                parse::PowerFlowDef::Constant {
                    rate: 30,
                    unit: parse::Unit("mw".to_string()),
                    time: parse::Unit("s".to_string()),
                },
            )])),
            ..Default::default()
        };
        let p2 = parse::NodeProfile {
            power_sources: Some(HashMap::from([(
                "solar".to_string(),
                parse::PowerFlowDef::Constant {
                    rate: 80,
                    unit: parse::Unit("mw".to_string()),
                    time: parse::Unit("s".to_string()),
                },
            )])),
            ..Default::default()
        };

        let mut node = parse::Node {
            resources: Some(parse::Resources {
                clock_rate: NonZeroU64::new(160),
                clock_units: Some(parse::Unit("mhz".to_string())),
                ..Default::default()
            }),
            power_sinks: Some(HashMap::from([(
                "mcu".to_string(),
                parse::PowerFlowDef::Constant {
                    rate: 50,
                    unit: parse::Unit("mw".to_string()),
                    time: parse::Unit("s".to_string()),
                },
            )])),
            ..Default::default()
        };

        apply_profile(&mut node, &p1);
        apply_profile(&mut node, &p2);

        let res = node.resources.unwrap();
        // User's clock_rate wins over p1.
        assert_eq!(res.clock_rate, NonZeroU64::new(160));
        // p1's ram fills the gap.
        assert_eq!(res.ram, NonZeroU64::new(520));
        // User's mcu sink wins over p1's.
        match &node.power_sinks.as_ref().unwrap()["mcu"] {
            parse::PowerFlowDef::Constant { rate, .. } => assert_eq!(*rate, 50),
            other => panic!("expected Constant, got {other:?}"),
        }
        // p2's solar source still applies.
        assert!(node.power_sources.as_ref().unwrap().contains_key("solar"));
    }

    #[test]
    fn multi_profile_channel_energy_layers() {
        let p1 = parse::NodeProfile {
            channel_energy: Some(HashMap::from([(
                "lora".to_string(),
                parse::ChannelEnergy {
                    tx: Some(parse::Energy {
                        quantity: 50,
                        unit: parse::Unit("uj".to_string()),
                    }),
                    rx: None,
                },
            )])),
            ..Default::default()
        };
        let p2 = parse::NodeProfile {
            channel_energy: Some(HashMap::from([
                (
                    "lora".to_string(),
                    parse::ChannelEnergy {
                        tx: Some(parse::Energy {
                            quantity: 999,
                            unit: parse::Unit("uj".to_string()),
                        }),
                        rx: Some(parse::Energy {
                            quantity: 10,
                            unit: parse::Unit("uj".to_string()),
                        }),
                    },
                ),
                (
                    "wifi".to_string(),
                    parse::ChannelEnergy {
                        tx: Some(parse::Energy {
                            quantity: 200,
                            unit: parse::Unit("uj".to_string()),
                        }),
                        rx: None,
                    },
                ),
            ])),
            ..Default::default()
        };

        let mut node = parse::Node::default();
        apply_profile(&mut node, &p1);
        apply_profile(&mut node, &p2);

        let ce = node.channel_energy.unwrap();
        // p1's "lora" wins over p2's (whole entry, not per-field).
        assert_eq!(ce["lora"].tx.as_ref().unwrap().quantity, 50);
        assert!(ce["lora"].rx.is_none());
        // p2's "wifi" fills the gap.
        assert_eq!(ce["wifi"].tx.as_ref().unwrap().quantity, 200);
    }

    #[test]
    fn multi_profile_empty_profile_is_noop() {
        let p1 = parse::NodeProfile {
            resources: Some(parse::Resources {
                clock_rate: NonZeroU64::new(240),
                clock_units: Some(parse::Unit("mhz".to_string())),
                ..Default::default()
            }),
            ..Default::default()
        };
        let empty = parse::NodeProfile::default();

        let mut node = parse::Node::default();
        apply_profile(&mut node, &p1);
        apply_profile(&mut node, &empty);

        let res = node.resources.unwrap();
        assert_eq!(res.clock_rate, NonZeroU64::new(240));
    }

    #[test]
    fn multi_profile_three_profiles_layer() {
        // Three profiles each contributing to disjoint maps.
        let board = parse::NodeProfile {
            resources: Some(parse::Resources {
                clock_rate: NonZeroU64::new(240),
                clock_units: Some(parse::Unit("mhz".to_string())),
                ram: NonZeroU64::new(520),
                ram_units: Some(parse::Unit("kb".to_string())),
                ..Default::default()
            }),
            power_states: Some(HashMap::from([(
                "active".to_string(),
                parse::PowerRate {
                    rate: 100,
                    unit: parse::Unit("mw".to_string()),
                    time: parse::Unit("s".to_string()),
                },
            )])),
            ..Default::default()
        };
        let solar = parse::NodeProfile {
            power_sources: Some(HashMap::from([(
                "solar".to_string(),
                parse::PowerFlowDef::Constant {
                    rate: 80,
                    unit: parse::Unit("mw".to_string()),
                    time: parse::Unit("s".to_string()),
                },
            )])),
            ..Default::default()
        };
        let radio = parse::NodeProfile {
            channel_energy: Some(HashMap::from([(
                "lora".to_string(),
                parse::ChannelEnergy {
                    tx: Some(parse::Energy {
                        quantity: 50,
                        unit: parse::Unit("uj".to_string()),
                    }),
                    rx: Some(parse::Energy {
                        quantity: 10,
                        unit: parse::Unit("uj".to_string()),
                    }),
                },
            )])),
            power_sinks: Some(HashMap::from([(
                "radio".to_string(),
                parse::PowerFlowDef::Constant {
                    rate: 20,
                    unit: parse::Unit("mw".to_string()),
                    time: parse::Unit("s".to_string()),
                },
            )])),
            ..Default::default()
        };

        let mut node = parse::Node::default();
        apply_profile(&mut node, &board);
        apply_profile(&mut node, &solar);
        apply_profile(&mut node, &radio);

        assert_eq!(node.resources.unwrap().clock_rate, NonZeroU64::new(240));
        assert!(node.power_states.as_ref().unwrap().contains_key("active"));
        assert!(node.power_sources.as_ref().unwrap().contains_key("solar"));
        assert!(node.power_sinks.as_ref().unwrap().contains_key("radio"));
        assert_eq!(
            node.channel_energy.as_ref().unwrap()["lora"]
                .tx
                .as_ref()
                .unwrap()
                .quantity,
            50
        );
    }

    #[test]
    fn multi_profile_resolve_and_merge_integration() {
        // End-to-end: two module files each providing a profile; a node
        // references both via `profile = ["board", "energy"]`.
        let dir = setup_temp_modules(&[
            (
                "hw.toml",
                "[profiles.board]\n\
                 [profiles.board.resources]\n\
                 clock_rate = 240\nclock_units = \"mhz\"\n\
                 ram = 520\nram_units = \"kb\"\n\
                 [profiles.board.power_states.active]\n\
                 rate = 100\nunit = \"mw\"\ntime = \"s\"\n",
            ),
            (
                "pwr.toml",
                "[profiles.energy.power_sources.solar]\n\
                 type = \"constant\"\nrate = 80\nunit = \"mw\"\ntime = \"s\"\n",
            ),
        ]);
        let mut sim = parse::Simulation::default();
        sim.r#use = Some(vec!["./hw".to_string(), "./pwr".to_string()]);
        resolve_and_merge(dir.path(), &mut sim).unwrap();

        let profiles = sim.profiles.as_ref().unwrap();
        assert!(profiles.contains_key("board"));
        assert!(profiles.contains_key("energy"));

        // Simulate profile application as validate() would.
        let mut node = parse::Node::default();
        apply_profile(&mut node, &profiles["board"]);
        apply_profile(&mut node, &profiles["energy"]);

        assert_eq!(node.resources.unwrap().clock_rate, NonZeroU64::new(240));
        assert!(node.power_states.as_ref().unwrap().contains_key("active"));
        assert!(node.power_sources.as_ref().unwrap().contains_key("solar"));
    }

    #[test]
    fn multi_profile_duplicate_profile_name_across_modules() {
        // Two modules both define a profile named "board" -- should error.
        let dir = setup_temp_modules(&[
            (
                "a.toml",
                "[profiles.board]\n\
                 [profiles.board.resources]\n\
                 clock_rate = 240\nclock_units = \"mhz\"\n",
            ),
            (
                "b.toml",
                "[profiles.board]\n\
                 [profiles.board.resources]\n\
                 clock_rate = 80\nclock_units = \"mhz\"\n",
            ),
        ]);
        let mut sim = parse::Simulation::default();
        sim.r#use = Some(vec!["./a".to_string(), "./b".to_string()]);
        let err = resolve_and_merge(dir.path(), &mut sim).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("Duplicate profile"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn multi_profile_layers_compose() {
        // First profile: board resources + power states + sink
        let board = parse::NodeProfile {
            resources: Some(parse::Resources {
                clock_rate: NonZeroU64::new(240),
                clock_units: Some(parse::Unit("mhz".to_string())),
                ram: NonZeroU64::new(520),
                ram_units: Some(parse::Unit("kb".to_string())),
                ..Default::default()
            }),
            power_states: Some(HashMap::from([(
                "active".to_string(),
                parse::PowerRate {
                    rate: 100,
                    unit: parse::Unit("mw".to_string()),
                    time: parse::Unit("s".to_string()),
                },
            )])),
            power_sinks: Some(HashMap::from([(
                "mcu".to_string(),
                parse::PowerFlowDef::Constant {
                    rate: 30,
                    unit: parse::Unit("mw".to_string()),
                    time: parse::Unit("s".to_string()),
                },
            )])),
            ..Default::default()
        };
        // Second profile: solar power source
        let solar = parse::NodeProfile {
            power_sources: Some(HashMap::from([(
                "solar".to_string(),
                parse::PowerFlowDef::Constant {
                    rate: 80,
                    unit: parse::Unit("mw".to_string()),
                    time: parse::Unit("s".to_string()),
                },
            )])),
            ..Default::default()
        };

        let mut node = parse::Node::default();
        apply_profile(&mut node, &board);
        apply_profile(&mut node, &solar);

        // Board resources applied.
        let res = node.resources.unwrap();
        assert_eq!(res.clock_rate, NonZeroU64::new(240));
        assert_eq!(res.ram, NonZeroU64::new(520));
        // Board power states applied.
        assert!(node.power_states.as_ref().unwrap().contains_key("active"));
        // Board sink applied.
        assert!(node.power_sinks.as_ref().unwrap().contains_key("mcu"));
        // Solar source layered on top.
        assert!(node.power_sources.as_ref().unwrap().contains_key("solar"));
    }
}
