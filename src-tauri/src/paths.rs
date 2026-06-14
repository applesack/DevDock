use std::path::PathBuf;

use anyhow::{Context, Result};
use directories::BaseDirs;

pub const APP_DIRECTORY_NAME: &str = "DevDock";
pub const CONFIG_FILE_NAME: &str = "devdock.config.json";

pub fn app_data_dir() -> Result<PathBuf> {
    let base_dirs = BaseDirs::new().context("cannot determine the Windows AppData directory")?;
    Ok(base_dirs.config_dir().to_path_buf())
}

pub fn config_dir() -> Result<PathBuf> {
    Ok(app_data_dir()?.join(APP_DIRECTORY_NAME))
}

pub fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join(CONFIG_FILE_NAME))
}
