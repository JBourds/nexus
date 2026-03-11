use anyhow::{Context, Result, bail};
use crc32fast::Hasher;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

mod channel;
mod helpers;
mod medium;
pub mod module;
mod namespace;
pub mod parse;
mod position;
mod resources;
mod time;
mod units;
mod validate;

pub mod ast;

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub const CONTROL_PREFIX: &str = "ctl.";
const RESERVED_LINKS: [&str; 1] = ["ideal"];

pub fn parse(mut config_root: PathBuf) -> Result<ast::Simulation> {
    let config_text = std::fs::read_to_string(&config_root).context(format!(
        "Unable to open file located at {}",
        config_root.to_string_lossy()
    ))?;
    let mut parsed: parse::Simulation = toml::from_str(config_text.as_str())
        .context("Failed to parse simulation parameters from config file.")?;
    config_root.pop();

    // Resolve module imports and merge into the parsed simulation.
    module::resolve_and_merge(&config_root, &mut parsed)
        .context("Failed to resolve module imports.")?;

    let validated = ast::Simulation::validate(&config_root, parsed)
        .context("Failed to validate simulation parameters from config file.")?;
    Ok(validated)
}

#[derive(Serialize, Deserialize, Debug)]
struct ConfigSnapshot {
    cfg: String,
    version: String,
    checksum: u32,
}

impl ConfigSnapshot {
    fn new(sim: &ast::Simulation) -> Self {
        let cfg = toml::to_string_pretty(sim).expect("unable to serialize toml to string");
        let mut hasher = Hasher::new();
        hasher.update(cfg.as_bytes());
        let checksum = hasher.finalize();
        Self {
            cfg,
            version: VERSION.to_string(),
            checksum,
        }
    }

    fn try_read(path: &Path) -> Result<Self> {
        let config_text = std::fs::read_to_string(path).context(format!(
            "Unable to open file located at {}",
            path.to_string_lossy()
        ))?;
        let cfg: ConfigSnapshot = toml::from_str(config_text.as_str())
            .context("Failed to parse config snapshot from path.")?;
        let checksum = {
            let mut hasher = Hasher::new();
            hasher.update(cfg.cfg.as_bytes());
            hasher.finalize()
        };
        if checksum != cfg.checksum {
            bail!("Checksum mismatch for configuration file!");
        }
        Ok(cfg)
    }
}

pub fn serialize_config(sim: &ast::Simulation, dest: &Path) -> Result<()> {
    let cfg = ConfigSnapshot::new(sim);
    let s = toml::to_string_pretty(&cfg)?;
    std::fs::write(dest, s.as_bytes())?;
    Ok(())
}

/// Extract the raw `use` list and per-node profile assignments from TOML text
/// before module resolution consumes them.
pub fn extract_module_info(
    toml_text: &str,
) -> (Vec<String>, std::collections::HashMap<String, Vec<String>>) {
    let mut use_list = Vec::new();
    let mut node_profiles = std::collections::HashMap::new();

    if let Ok(val) = toml_text.parse::<toml::Table>() {
        if let Some(toml::Value::Array(arr)) = val.get("use") {
            for v in arr {
                if let toml::Value::String(s) = v {
                    use_list.push(s.clone());
                }
            }
        }
        if let Some(toml::Value::Table(nodes)) = val.get("nodes") {
            for (name, node_val) in nodes {
                if let toml::Value::Table(node_tbl) = node_val
                    && let Some(profile_val) = node_tbl.get("profile") {
                        let profiles = match profile_val {
                            toml::Value::String(s) => vec![s.clone()],
                            toml::Value::Array(arr) => arr
                                .iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect(),
                            _ => Vec::new(),
                        };
                        if !profiles.is_empty() {
                            node_profiles.insert(name.clone(), profiles);
                        }
                    }
            }
        }
    }

    (use_list, node_profiles)
}

/// Parse a module file from a path, returning its structured contents.
pub fn parse_module_file(path: &Path) -> Result<parse::ModuleFile> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("Unable to read module file at \"{}\"", path.display()))?;
    let module: parse::ModuleFile = toml::from_str(&text)
        .with_context(|| format!("Failed to parse module file at \"{}\"", path.display()))?;
    Ok(module)
}

pub fn deserialize_config(src: &Path) -> Result<ast::Simulation> {
    let snapshot = ConfigSnapshot::try_read(src)?;
    if let Ok(res) = toml::from_str(snapshot.cfg.as_str()) {
        Ok(res)
    } else {
        bail!("Unable to deserialize validated configuration from snapshot.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::fs;

    #[test]
    fn rejects() {
        for entry in fs::read_dir("tests/rejects").unwrap() {
            let path = entry.unwrap().path();
            let config = path.join("nexus.toml");
            let expected = fs::read_to_string(path.join("expected.txt")).unwrap();
            let expected = expected.trim_end();
            let res = parse(config);
            let msg = format!(
                "Expected {path:?} to be rejected with error:\n\n\"{expected}\"\
                \n\nBut got result:\n\n\"{res:#?}\""
            );
            assert_eq!(
                res.is_err_and(|e| format!("{e:#?}") == expected),
                true,
                "{msg}"
            );
        }
    }

    #[test]
    fn accepts() {
        for entry in fs::read_dir("tests/accepts").unwrap() {
            let path = entry.unwrap().path();
            let res = parse(path.clone());
            assert_eq!(
                res.is_ok(),
                true,
                "Expected {path:?} to be accepted but got result:\n{res:#?}"
            );
        }
    }

    #[test]
    fn extract_module_info_with_use_and_profiles() {
        let toml = r#"
use = ["lora/sx1276_915mhz", "boards/esp32_devkit"]

[params]
seed = 42

[nodes.sensor]
profile = ["esp32", "solar_small"]

[nodes.gateway]
profile = "esp32"

[nodes.plain]
"#;
        let (use_list, node_profiles) = extract_module_info(toml);
        assert_eq!(use_list, vec!["lora/sx1276_915mhz", "boards/esp32_devkit"]);
        assert_eq!(
            node_profiles.get("sensor").unwrap(),
            &vec!["esp32".to_string(), "solar_small".to_string()]
        );
        assert_eq!(
            node_profiles.get("gateway").unwrap(),
            &vec!["esp32".to_string()]
        );
        assert!(node_profiles.get("plain").is_none());
    }

    #[test]
    fn extract_module_info_empty() {
        let (use_list, node_profiles) = extract_module_info("[params]\nseed = 1\n");
        assert!(use_list.is_empty());
        assert!(node_profiles.is_empty());
    }

    #[test]
    fn extract_module_info_invalid_toml() {
        let (use_list, node_profiles) = extract_module_info("not valid { toml");
        assert!(use_list.is_empty());
        assert!(node_profiles.is_empty());
    }
}
