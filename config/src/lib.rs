use anyhow::{Context, Result, bail};
use crc32fast::Hasher;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

mod helpers;
mod namespace;
pub(crate) mod parse;
mod units;
mod validate;

pub mod ast;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const CONTROL_FILES: [&str; 4] = ["time", "energy_state", "energy_left", "position"];
const RESERVED_LINKS: [&str; 1] = ["ideal"];

pub fn parse(mut config_root: PathBuf) -> Result<ast::Simulation> {
    let config_text = std::fs::read_to_string(&config_root).context(format!(
        "Unable to open file located at {}",
        config_root.to_string_lossy()
    ))?;
    let parsed: parse::Simulation = toml::from_str(config_text.as_str())
        .context("Failed to parse simulation parameters from config file.")?;
    config_root.pop();
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
}
