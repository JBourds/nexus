use std::path::PathBuf;

use anyhow::{Context, Result};

mod helpers;
pub(crate) mod parse;
mod units;
mod validate;

pub mod ast;

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
