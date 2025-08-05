use std::path::PathBuf;

use anyhow::{Context, Result};

pub(crate) mod parse;
mod validate;

pub mod ast {
    pub use crate::validate::*;
}

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
    println!("{validated:#?}");
    Ok(validated)
}
