//! File-based tracing setup.
//!
//! The TUI owns the terminal, so logs must never go to stdout/stderr — we
//! write to `$XDG_CACHE_HOME/wiim-tui/log.txt` (typically
//! `~/.cache/wiim-tui/log.txt`). Falls back to a system temp file if the
//! XDG lookup fails (e.g. headless container without HOME).

use std::fs;
use std::path::PathBuf;

use color_eyre::eyre::{Context, Result, eyre};
use directories::ProjectDirs;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::writer::MakeWriterExt;

/// Initialise tracing. The returned guard must be kept alive for the
/// lifetime of the program — when it drops, buffered log lines flush.
pub fn init() -> Result<WorkerGuard> {
    let path = log_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating log dir at {}", parent.display()))?;
    }

    // Append rather than rotate; the file is small and rotation is more
    // complexity than this phase warrants.
    let file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("opening log file at {}", path.display()))?;

    let (writer, guard) = tracing_appender::non_blocking(file);

    // RUST_LOG overrides the default; "info" is plenty for now.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer.with_max_level(tracing::Level::TRACE))
        .with_ansi(false)
        .with_target(false)
        .init();

    Ok(guard)
}

fn log_path() -> Result<PathBuf> {
    if let Some(dirs) = ProjectDirs::from("", "", "wiim-tui") {
        return Ok(dirs.cache_dir().join("log.txt"));
    }
    // Fallback: system temp dir. Only hit on exotic setups where
    // ProjectDirs can't determine $HOME.
    let mut path = std::env::temp_dir();
    if !path.exists() {
        return Err(eyre!("no usable cache dir and temp_dir does not exist"));
    }
    path.push("wiim-tui");
    path.push("log.txt");
    Ok(path)
}
