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
    /// `None` only when the user cancels.
    ///
    /// A failed write is *not* a cancellation. The reader picked a folder, so
    /// the session opens it either way and the failure is reported rather than
    /// swallowed — returning `None` here would leave the splash up with no
    /// explanation, which is exactly what a missing home directory used to do.
    pub fn onboard() -> Option<Config> {
        let mut dialog = rfd::FileDialog::new().set_title("Choose a vault folder");
        if let Some(home) = home() {
            dialog = dialog.set_directory(home);
        }
        let config = Config {
            vault: dialog.pick_folder()?,
        };
        if let Err(e) = config.save() {
            eprintln!("could not save the vault config: {e}");
        }
        Some(config)
    }
}

/// The user's home directory. `std::env::home_dir` is the cross-platform
/// answer: `$HOME` then the passwd entry on Unix, `USERPROFILE` then
/// `FOLDERID_Profile` on Windows. Reading `$HOME` directly is a Unix-only
/// lookup — on Windows it is unset, and every path built from it vanished.
fn home() -> Option<PathBuf> {
    std::env::home_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The onboarding bug this guards: `home()` read `$HOME` directly, which
    /// is unset on Windows, so `dir()` was `None`, `save()` failed, and
    /// `onboard()` returned `None` — indistinguishable from the reader
    /// cancelling the folder dialog. The splash stayed up forever.
    #[test]
    fn a_config_path_resolves_on_this_platform() {
        let file = Config::file().expect("no config path on this platform");
        assert!(file.is_absolute(), "config path must be absolute: {file:?}");
        assert!(file.ends_with("config.toml"));
    }

    #[test]
    fn a_config_round_trips_through_toml() {
        let config = Config {
            vault: PathBuf::from("vault"),
        };
        let text = toml::to_string_pretty(&config).unwrap();
        assert_eq!(toml::from_str::<Config>(&text).unwrap(), config);
    }
}
