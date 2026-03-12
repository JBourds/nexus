use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

pub(crate) fn expand_home(path: &PathBuf) -> PathBuf {
    if let Some(stripped) = path.to_string_lossy().strip_prefix("~/")
        && let Some(home_dir) = home::home_dir()
    {
        return home_dir.join(stripped);
    }
    PathBuf::from(path)
}

pub(crate) fn resolve_directory(config_root: &PathBuf, path: &PathBuf) -> Result<PathBuf> {
    let root = expand_home(path);
    let root = if root.is_relative() {
        std::fs::canonicalize(Path::new(config_root).join(&root))
            .with_context(|| format!("Cannot resolve path \"{}\"", root.display()))?
    } else {
        root
    };
    match root.try_exists() {
        Ok(true) => {}
        Ok(false) => {
            bail!(
                "Unable to find directory at path \"{}\"",
                path.to_string_lossy()
            );
        }
        err => {
            err.context(format!(
                "Could not verify whether root exists at path \"{}\"",
                root.to_string_lossy()
            ))?;
        }
    }
    if !root.is_dir() {
        bail!("Protocol root at \"{}\" is not a directory", root.display());
    } else {
        Ok(root)
    }
}
