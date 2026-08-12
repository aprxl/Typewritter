//! Vault configuration: `~/.typewritter/config.toml`.
//!
//! First launch creates the directory and asks the user for a vault;
//! every launch after that reads the file and opens the vault directly.
//! `--onboard` (parsed in `main`) discards the saved config and repeats
//! the prompt.

use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const CONFIG_DIR: &str = ".typewritter";
const CONFIG_FILE: &str = "config.toml";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    pub vault: PathBuf,
}

impl Config {
    /// `~/.typewritter`, or `None` when there is no home directory.
    pub fn dir() -> Option<PathBuf> {
        home().map(|home| home.join(CONFIG_DIR))
    }

    /// `~/.typewritter/config.toml`.
    pub fn file() -> Option<PathBuf> {
        Self::dir().map(|dir| dir.join(CONFIG_FILE))
    }

    /// Reads the saved config, if one exists and parses.
    pub fn load() -> Option<Config> {
        let text = fs::read_to_string(Self::file()?).ok()?;
        toml::from_str(&text).ok()
    }

    /// Writes the config to disk, creating `~/.typewritter` first.
    pub fn save(&self) -> io::Result<()> {
        let dir = Self::dir().ok_or_else(|| io::Error::other("no home directory"))?;
        fs::create_dir_all(&dir)?;
        let text = toml::to_string_pretty(self).map_err(|e| io::Error::other(e.to_string()))?;
        fs::write(dir.join(CONFIG_FILE), text)
    }

    /// The first-run / `--onboard` flow: native folder dialog, persisted.
    /// `None` when the user cancels.
    pub fn onboard() -> Option<Config> {
        let vault = rfd::FileDialog::new()
            .set_title("Choose a vault folder")
            .set_directory(home().unwrap_or_else(|| PathBuf::from("/")))
            .pick_folder()?;
        let config = Config { vault };
        config.save().ok()?;
        Some(config)
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}
