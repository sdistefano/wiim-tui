//! `App` is the only mutable state. `dispatch` is the only function that
//! mutates it. This split keeps the logic unit-testable — feed Actions
//! in, assert on the resulting state, no tokio or terminal needed.

use crossterm::event::KeyCode;
use tokio::sync::mpsc;
use tracing::debug;
use wiim_api::{NowPlaying, PlayState};

use crate::action::Action;
use crate::commands::Command;
use crate::source::{Source, next_in_cycle};

/// Volume step for `+` / `-`. `=` uses 1 for fine adjustment.
const VOLUME_STEP: u8 = 5;

/// Top-level view. Not many of these yet — `NowPlaying` is the default,
/// `Queue` is a thin Phase-3 view. Phase 4 will add `Eq`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    NowPlaying,
    Queue,
}

impl View {
    /// Cycle order for Tab / BackTab. Keep this stable; the user builds
    /// muscle memory around it.
    const ORDER: &'static [View] = &[View::NowPlaying, View::Queue];

    fn step(self, delta: i32) -> Self {
        let idx = Self::ORDER.iter().position(|v| *v == self).unwrap_or(0) as i32;
        let len = Self::ORDER.len() as i32;
        let next = (idx + delta).rem_euclid(len) as usize;
        Self::ORDER[next]
    }
}

pub struct App {
    pub device_ip: String,
    pub now_playing: Option<NowPlaying>,
    pub source: Source,
    pub view: View,
    /// One-line error surfaced at the bottom of the UI. `Esc` clears it.
    pub error: Option<String>,
    pub should_quit: bool,
    /// Channel to the command worker. `None` only in tests.
    cmd_tx: Option<mpsc::UnboundedSender<Command>>,
}

impl App {
    pub fn new(device_ip: String, cmd_tx: mpsc::UnboundedSender<Command>) -> Self {
        Self {
            device_ip,
            now_playing: None,
            source: Source::Unknown,
            view: View::NowPlaying,
            error: None,
            should_quit: false,
            cmd_tx: Some(cmd_tx),
        }
    }

    /// Test-only: build an App with no command channel. Key actions that
    /// would dispatch a command silently no-op, which is fine for unit
    /// tests focused on state transitions.
    #[cfg(test)]
    pub fn for_test(device_ip: String) -> Self {
        Self {
            device_ip,
            now_playing: None,
            source: Source::Unknown,
            view: View::NowPlaying,
            error: None,
            should_quit: false,
            cmd_tx: None,
        }
    }

    fn send(&self, cmd: Command) {
        let Some(tx) = &self.cmd_tx else {
            debug!(?cmd, "cmd_tx unset (test mode), dropping command");
            return;
        };
        if tx.send(cmd).is_err() {
            tracing::error!("command worker channel closed");
        }
    }
}

/// Single mutation entry point. Pure-ish: side effects are limited to
/// sending a `Command` on the worker channel, never to terminal IO.
pub fn dispatch(app: &mut App, action: Action) {
    match action {
        Action::ApiError(msg) => {
            app.error = Some(msg);
        }
        Action::Refresh(snap) => {
            // A successful refresh implicitly means the device is reachable.
            // Don't auto-clear errors here; the user dismisses with Esc.
            app.now_playing = Some(snap.now_playing);
            app.source = snap.source;
        }
        Action::Key(code) => handle_key(app, code),
    }
}

fn handle_key(app: &mut App, code: KeyCode) {
    match code {
        // Quit: uppercase Q only — lowercase q is the queue view.
        KeyCode::Char('Q') => app.should_quit = true,
        KeyCode::Esc => app.error = None,

        // View switching
        KeyCode::Tab => app.view = app.view.step(1),
        KeyCode::BackTab => app.view = app.view.step(-1),
        KeyCode::Char('n') => app.view = View::NowPlaying,
        KeyCode::Char('q') => app.view = View::Queue,

        // Transport
        KeyCode::Char(' ') => toggle_play(app),
        KeyCode::Char('+') => bump_volume(app, VOLUME_STEP as i16),
        KeyCode::Char('-') => bump_volume(app, -(VOLUME_STEP as i16)),
        KeyCode::Char('=') => bump_volume(app, 1),
        KeyCode::Char('m') => toggle_mute(app),
        KeyCode::Char('>') => app.send(Command::NextTrack),
        KeyCode::Char('<') => app.send(Command::PrevTrack),

        // Source cycling — optimistically update the local label so the
        // UI reacts. Next poll reconciles if the device rejected.
        KeyCode::Char('s') => cycle_source(app),

        // Radio presets 1-9. Slot 0 is reserved (LinkPlay convention)
        // and not exposed.
        KeyCode::Char(c @ '1'..='9') => {
            let n = c.to_digit(10).unwrap() as u8;
            app.send(Command::PlayPreset(n));
        }

        other => debug!(?other, "unbound key"),
    }
}

fn toggle_play(app: &mut App) {
    // Optimistic: flip the local state so the UI reacts instantly. Next
    // poll will reconcile if the device disagrees.
    if let Some(np) = app.now_playing.as_mut() {
        np.state = match np.state {
            PlayState::Playing | PlayState::Loading => PlayState::Paused,
            PlayState::Paused | PlayState::Stopped => PlayState::Playing,
        };
    }
    app.send(Command::TogglePlay);
}

fn bump_volume(app: &mut App, delta: i16) {
    let Some(np) = app.now_playing.as_mut() else {
        return;
    };
    let new = (np.volume as i16 + delta).clamp(0, 100) as u8;
    if new == np.volume {
        return;
    }
    np.volume = new;
    app.send(Command::SetVolume(new));
}

fn toggle_mute(app: &mut App) {
    let Some(np) = app.now_playing.as_mut() else {
        return;
    };
    np.is_muted = !np.is_muted;
    let cmd = if np.is_muted {
        Command::Mute
    } else {
        Command::Unmute
    };
    app.send(cmd);
}

fn cycle_source(app: &mut App) {
    let next = next_in_cycle(app.source);
    if next.switch_arg().is_none() {
        // Defensive — `next_in_cycle` only returns sources from a fixed
        // list, all of which currently have a switch arg. If this ever
        // changes, surface a clear error rather than dropping silently.
        app.error = Some(format!("cannot switch to {:?}", next));
        return;
    }
    app.source = next; // optimistic
    app.send(Command::SwitchSource(next));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Snapshot;

    fn snap(state: PlayState, volume: u8, source: Source) -> Snapshot {
        Snapshot {
            now_playing: NowPlaying {
                title: Some("t".into()),
                artist: Some("a".into()),
                album: None,
                album_art_uri: None,
                state,
                volume,
                is_muted: false,
                position_ms: 0,
                duration_ms: 100_000,
                sample_rate: None,
                bit_depth: None,
            },
            source,
        }
    }

    #[test]
    fn uppercase_q_quits_lowercase_does_not() {
        let mut app = App::for_test("x".into());
        dispatch(&mut app, Action::Key(KeyCode::Char('q')));
        assert!(!app.should_quit, "lowercase q must not quit");
        // ...but does switch view
        assert_eq!(app.view, View::Queue);
        dispatch(&mut app, Action::Key(KeyCode::Char('Q')));
        assert!(app.should_quit);
    }

    #[test]
    fn esc_dismisses_error() {
        let mut app = App::for_test("x".into());
        app.error = Some("boom".into());
        dispatch(&mut app, Action::Key(KeyCode::Esc));
        assert!(app.error.is_none());
    }

    #[test]
    fn space_toggles_play_state_optimistically() {
        let mut app = App::for_test("x".into());
        dispatch(
            &mut app,
            Action::Refresh(snap(PlayState::Playing, 50, Source::Network)),
        );
        dispatch(&mut app, Action::Key(KeyCode::Char(' ')));
        assert!(matches!(
            app.now_playing.as_ref().unwrap().state,
            PlayState::Paused
        ));
        dispatch(&mut app, Action::Key(KeyCode::Char(' ')));
        assert!(matches!(
            app.now_playing.as_ref().unwrap().state,
            PlayState::Playing
        ));
    }

    #[test]
    fn volume_bump_clamps_at_bounds() {
        let mut app = App::for_test("x".into());
        dispatch(
            &mut app,
            Action::Refresh(snap(PlayState::Paused, 98, Source::Network)),
        );
        dispatch(&mut app, Action::Key(KeyCode::Char('+')));
        assert_eq!(app.now_playing.as_ref().unwrap().volume, 100);
        dispatch(&mut app, Action::Key(KeyCode::Char('+')));
        assert_eq!(app.now_playing.as_ref().unwrap().volume, 100);

        app.now_playing.as_mut().unwrap().volume = 2;
        dispatch(&mut app, Action::Key(KeyCode::Char('-')));
        assert_eq!(app.now_playing.as_ref().unwrap().volume, 0);
    }

    #[test]
    fn fine_volume_adjusts_by_one() {
        let mut app = App::for_test("x".into());
        dispatch(
            &mut app,
            Action::Refresh(snap(PlayState::Paused, 50, Source::Network)),
        );
        dispatch(&mut app, Action::Key(KeyCode::Char('=')));
        assert_eq!(app.now_playing.as_ref().unwrap().volume, 51);
    }

    #[test]
    fn mute_toggles_local_flag() {
        let mut app = App::for_test("x".into());
        dispatch(
            &mut app,
            Action::Refresh(snap(PlayState::Paused, 50, Source::Network)),
        );
        dispatch(&mut app, Action::Key(KeyCode::Char('m')));
        assert!(app.now_playing.as_ref().unwrap().is_muted);
        dispatch(&mut app, Action::Key(KeyCode::Char('m')));
        assert!(!app.now_playing.as_ref().unwrap().is_muted);
    }

    #[test]
    fn refresh_replaces_snapshot() {
        let mut app = App::for_test("x".into());
        dispatch(
            &mut app,
            Action::Refresh(snap(PlayState::Playing, 33, Source::Bluetooth)),
        );
        assert_eq!(app.now_playing.as_ref().unwrap().volume, 33);
        assert_eq!(app.source, Source::Bluetooth);
    }

    #[test]
    fn tab_cycles_views() {
        let mut app = App::for_test("x".into());
        assert_eq!(app.view, View::NowPlaying);
        dispatch(&mut app, Action::Key(KeyCode::Tab));
        assert_eq!(app.view, View::Queue);
        dispatch(&mut app, Action::Key(KeyCode::Tab));
        assert_eq!(app.view, View::NowPlaying);
        dispatch(&mut app, Action::Key(KeyCode::BackTab));
        assert_eq!(app.view, View::Queue);
    }

    #[test]
    fn n_and_q_jump_directly_to_views() {
        let mut app = App::for_test("x".into());
        dispatch(&mut app, Action::Key(KeyCode::Char('q')));
        assert_eq!(app.view, View::Queue);
        dispatch(&mut app, Action::Key(KeyCode::Char('n')));
        assert_eq!(app.view, View::NowPlaying);
    }

    #[test]
    fn s_cycles_source_optimistically() {
        let mut app = App::for_test("x".into());
        app.source = Source::Network;
        dispatch(&mut app, Action::Key(KeyCode::Char('s')));
        assert_eq!(app.source, Source::Bluetooth);
        dispatch(&mut app, Action::Key(KeyCode::Char('s')));
        assert_eq!(app.source, Source::LineIn);
    }
}
