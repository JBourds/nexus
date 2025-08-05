use anyhow::{Context, Result};

pub(crate) mod parse;
mod validate;

pub use validate::*;

pub fn parse(text: String) -> Result<Simulation> {
    let parsed: parse::Simulation = toml::from_str(text.as_str())
        .context("Failed to parse simulation parameters from config file.")?;
    let validated =
        validate(parsed).context("Failed to validate simulation parameters from config file.")?;
    println!("{validated:#?}");
    Ok(validated)
}
