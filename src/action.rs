//! `Action` is the single mutation channel for app state. Keystrokes,
//! poll results, and async errors all become Actions and flow through
//! the same `dispatch` path. This keeps state changes traceable and
//! testable.

use crossterm::event::KeyCode;

use crate::source::Snapshot;

#[derive(Debug)]
pub enum Action {
    /// User pressed a key. Mapped to a higher-level action by the
    /// dispatcher; we keep the raw key around so logging / unbound
    /// keys stay debuggable.
    Key(KeyCode),

    /// Polling task delivered a fresh state snapshot.
    Refresh(Snapshot),

    /// A command spawned by the user failed. Surfaced as a one-line
    /// error in the UI; the underlying error is in the trace log.
    ApiError(String),
}
