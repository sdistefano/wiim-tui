//! On-disk config. Tiny — currently just a default device IP.
//!
//! Path: `$XDG_CONFIG_HOME/wiim-tui/config.toml` (typically
//! `~/.config/wiim-tui/config.toml`). The file is optional; missing or
//! empty is fine. Parse errors are surfaced to the user — a malformed
//! config shouldn't silently fall back to "no device".

use std::fs;
use std::path::PathBuf;

use color_eyre::eyre::{Context, Result, eyre};
use directories::ProjectDirs;
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
pub struct Config {
    /// Default device IP (or `https://<host>` URL). Overridden by `--device`.
    pub device: Option<String>,
}

impl Config {
    /// Load config from disk. Returns an empty `Config` if the file
    /// doesn't exist; propagates IO and parse errors otherwise.
    pub fn load() -> Result<Self> {
        let Some(path) = config_path() else {
            return Ok(Self::default());
        };
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let cfg: Self =
            toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
        Ok(cfg)
    }
}

/// Path to `config.toml`. `None` only on exotic systems where
/// `ProjectDirs` can't determine a home directory.
pub fn config_path() -> Option<PathBuf> {
    ProjectDirs::from("", "", "wiim-tui").map(|d| d.config_dir().join("config.toml"))
}

/// Resolve the device to use for this session. CLI flag wins; otherwise
/// fall back to config; otherwise error with a hint pointing at the
/// config path.
pub fn resolve_device(cli_device: Option<String>, cfg: &Config) -> Result<String> {
    if let Some(d) = cli_device {
        return Ok(d);
    }
    if let Some(d) = cfg.device.clone() {
        return Ok(d);
    }
    let hint = config_path()
        .map(|p| format!(" or set `device = \"…\"` in {}", p.display()))
        .unwrap_or_default();
    Err(eyre!("no device specified — pass `--device <IP>`{hint}"))
}
