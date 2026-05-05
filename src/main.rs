//! wiim-tui — keyboard-driven terminal UI for WiiM (LinkPlay) audio streamers.
//!
//! Phase 1: control a single device by IP via `--device`. Polls every 1 s
//! and renders a NowPlaying view. Space toggles play/pause; `+`/`-`
//! adjust volume; `m` mutes; `>`/`<` skip; `Q` quits.

use std::io::{self, Stdout};
use std::panic;
use std::sync::Arc;

use clap::Parser;
use color_eyre::eyre::{Context, Result};
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    poll as event_poll, read as event_read,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;
use tracing::{error, info, instrument};
use wiim_api::WiimClient;

mod action;
mod app;
mod commands;
mod config;
mod logging;
mod raw;
mod source;
mod ui;

use action::Action;
use app::{App, dispatch};
use commands::{spawn_command_worker, spawn_poll};
use config::{Config, resolve_device};
use raw::RawClient;

type Tui = Terminal<CrosstermBackend<Stdout>>;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    /// IP address of the WiiM device to control. Falls back to
    /// `device` in `~/.config/wiim-tui/config.toml`. mDNS auto-discovery
    /// arrives in Phase 2.
    #[arg(long)]
    device: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let _guard = logging::init().context("setting up file tracing")?;
    let cli = Cli::parse();

    // Resolve the device before touching the terminal. Failing here
    // means we never put the terminal in raw mode, so the user sees a
    // normal error message rather than a corrupted shell.
    let cfg = Config::load().context("loading config")?;
    let device = resolve_device(cli.device, &cfg)?;
    info!(%device, "wiim-tui starting");

    install_panic_hook();

    let mut terminal = setup_terminal().context("setting up terminal")?;
    let result = run(&mut terminal, device).await;
    restore_terminal(&mut terminal).context("restoring terminal")?;

    if let Err(err) = &result {
        error!(?err, "exited with error");
    }
    info!("wiim-tui exiting");
    result
}

#[instrument(skip_all, fields(device = %device))]
async fn run(terminal: &mut Tui, device: String) -> Result<()> {
    // Don't .connect() here — we want the UI up immediately even if the
    // device is briefly offline. The first poll surfaces any failure as
    // an in-app error.
    let api = Arc::new(WiimClient::new(&device));
    // Phase 3 endpoints (switchmode, MCUKeyShortClick) aren't exposed by
    // wiim_api 0.1, so we run our own thin reqwest client alongside.
    let raw = Arc::new(RawClient::new(&device));

    let (action_tx, mut action_rx) = mpsc::unbounded_channel::<Action>();

    let cmd_tx = spawn_command_worker(api.clone(), raw.clone(), action_tx.clone());
    spawn_poll(api.clone(), action_tx.clone());

    let mut app = App::new(device, cmd_tx);

    let input_tx = action_tx.clone();
    let input_handle = tokio::task::spawn_blocking(move || input_loop(input_tx));

    // Render once before waiting on the channel so the UI shows up even
    // if the first poll is still in flight.
    terminal.draw(|f| ui::render(&app, f))?;

    while let Some(action) = action_rx.recv().await {
        dispatch(&mut app, action);
        if app.should_quit {
            break;
        }
        terminal.draw(|f| ui::render(&app, f))?;
    }

    // Close the channel from the receiver side so all senders' next
    // operation fails. The input thread checks `is_closed()` after each
    // poll tick and exits within the poll interval. Without this the
    // `await` below would hang forever — the input thread has its own
    // sender clone, so dropping our `action_tx` alone is not enough.
    drop(action_rx);
    drop(action_tx);
    let _ = input_handle.await;
    Ok(())
}

/// Sync input pump on a blocking thread. Each key press becomes an
/// `Action::Key` for the dispatcher; Ctrl+C is rewritten to a Q so the
/// terminal restores cleanly even if the user reflexively hits it.
///
/// Exits when the receiver side of `tx` is dropped — the main loop
/// signals shutdown by dropping `action_rx`, which makes
/// `tx.is_closed()` return true on the next poll tick.
fn input_loop(tx: mpsc::UnboundedSender<Action>) {
    use std::time::Duration;

    loop {
        if tx.is_closed() {
            break;
        }

        match event_poll(Duration::from_millis(250)) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(err) => {
                error!(?err, "crossterm poll failed");
                break;
            }
        }

        let event = match event_read() {
            Ok(ev) => ev,
            Err(err) => {
                error!(?err, "crossterm read failed");
                break;
            }
        };

        let Event::Key(key) = event else { continue };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        // Ctrl+C → quit. In raw mode, the OS does not turn it into
        // SIGINT; we have to handle it ourselves or the user gets stuck.
        let code =
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                KeyCode::Char('Q')
            } else {
                key.code
            };

        if tx.send(Action::Key(code)).is_err() {
            break;
        }
    }
}

fn setup_terminal() -> Result<Tui> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    Ok(Terminal::new(backend)?)
}

fn restore_terminal(terminal: &mut Tui) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

/// Restore terminal state on panic so the user isn't left with a
/// scrambled shell. The original hook still runs for the backtrace.
fn install_panic_hook() {
    let original = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        original(info);
    }));
}
