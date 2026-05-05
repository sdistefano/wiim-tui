//! Async workers that talk to the device. Two tasks live here:
//!
//! - `spawn_poll` — ticks every `POLL_INTERVAL` and emits `Action::Refresh`.
//! - `spawn_command_worker` — serialises user commands (play/pause/volume/...)
//!   so that rapid keypresses can't race each other on the device side.

use std::sync::Arc;
use std::time::Duration;

use color_eyre::eyre::{Context, Result, eyre};
use tokio::sync::mpsc;
use tracing::{debug, warn};
use wiim_api::{NowPlaying, PlayState, PlayerStatus, WiimClient};

use crate::action::Action;
use crate::raw::RawClient;
use crate::source::{Snapshot, Source};

/// 1 s matches the plan: faster is wasteful on a LAN, slower feels laggy.
pub const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// User-driven commands. Each variant maps to one or two device methods.
/// Routed through a single worker task so they execute in order, even if
/// the user mashes keys.
#[derive(Debug, Clone)]
pub enum Command {
    TogglePlay,
    SetVolume(u8),
    Mute,
    Unmute,
    NextTrack,
    PrevTrack,
    /// Switch to the given input source. No-op if the source can't be
    /// switched to (e.g. `Unknown`); error surfaced if the device rejects.
    SwitchSource(Source),
    /// Fire `MCUKeyShortClick:N` — play preset N (1-based, typically 1-9).
    PlayPreset(u8),
}

/// Spawn the polling loop. Lives until `action_tx` is dropped (i.e. the
/// app is shutting down). Builds a `Snapshot` from `getPlayerStatus +
/// getMetaInfo` directly rather than calling `get_now_playing()` —
/// the latter would do `getPlayerStatus` twice (we need the `mode`
/// field for source detection).
pub fn spawn_poll(client: Arc<WiimClient>, action_tx: mpsc::UnboundedSender<Action>) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(POLL_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            match fetch_snapshot(&client).await {
                Ok(snap) => {
                    if action_tx.send(Action::Refresh(snap)).is_err() {
                        debug!("poll: action channel closed, exiting");
                        return;
                    }
                }
                Err(err) => {
                    warn!(?err, "poll: fetch_snapshot failed");
                    let msg = format!("poll failed: {err}");
                    if action_tx.send(Action::ApiError(msg)).is_err() {
                        return;
                    }
                }
            }
        }
    });
}

async fn fetch_snapshot(client: &WiimClient) -> Result<Snapshot> {
    let (status, meta) = tokio::try_join!(client.get_player_status(), client.get_meta_info())
        .context("getPlayerStatus + getMetaInfo")?;
    let np = build_now_playing(&status, &meta)?;
    let source = Source::from_status(&status);
    Ok(Snapshot {
        now_playing: np,
        source,
    })
}

/// Mirror of `WiimClient::get_now_playing`'s post-fetch work, lifted
/// here so we can reuse the same `PlayerStatus` for source detection.
fn build_now_playing(status: &PlayerStatus, meta: &wiim_api::MetaInfo) -> Result<NowPlaying> {
    let state = match status.status.as_str() {
        "play" => PlayState::Playing,
        "pause" => PlayState::Paused,
        "stop" => PlayState::Stopped,
        "loading" => PlayState::Loading,
        _ => PlayState::Stopped,
    };
    let volume = status
        .vol
        .parse::<u8>()
        .map_err(|_| eyre!("invalid volume: {}", status.vol))?;
    let position_ms = status
        .curpos
        .parse::<u64>()
        .map_err(|_| eyre!("invalid position: {}", status.curpos))?;
    let duration_ms = status
        .totlen
        .parse::<u64>()
        .map_err(|_| eyre!("invalid duration: {}", status.totlen))?;
    Ok(NowPlaying {
        title: meta.meta_data.title.clone(),
        artist: meta.meta_data.artist.clone(),
        album: meta.meta_data.album.clone(),
        album_art_uri: meta.meta_data.album_art_uri.clone(),
        state,
        volume,
        is_muted: status.mute == "1",
        position_ms,
        duration_ms,
        sample_rate: meta.meta_data.sample_rate.clone(),
        bit_depth: meta.meta_data.bit_depth.clone(),
    })
}

/// Spawn the command worker and return the sender used to feed it.
/// One worker means one inflight HTTP call at a time, which keeps the
/// device's view of state monotonic w.r.t. user input order.
pub fn spawn_command_worker(
    api: Arc<WiimClient>,
    raw: Arc<RawClient>,
    action_tx: mpsc::UnboundedSender<Action>,
) -> mpsc::UnboundedSender<Command> {
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<Command>();

    tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            let label = format!("{cmd:?}");
            let res = run(&api, &raw, cmd).await;
            if let Err(err) = res {
                warn!(%label, ?err, "command failed");
                let msg = format!("{label}: {err}");
                if action_tx.send(Action::ApiError(msg)).is_err() {
                    return;
                }
            }
        }
    });

    cmd_tx
}

async fn run(api: &WiimClient, raw: &RawClient, cmd: Command) -> Result<()> {
    match cmd {
        Command::TogglePlay => api.toggle_play_pause().await.map_err(Into::into),
        Command::SetVolume(v) => api.set_volume(v).await.map_err(Into::into),
        Command::Mute => api.mute().await.map_err(Into::into),
        Command::Unmute => api.unmute().await.map_err(Into::into),
        Command::NextTrack => api.next_track().await.map_err(Into::into),
        Command::PrevTrack => api.previous_track().await.map_err(Into::into),
        Command::SwitchSource(s) => match s.switch_arg() {
            Some(arg) => raw.switch_mode(arg).await,
            None => Err(eyre!("source {:?} cannot be switched to", s)),
        },
        Command::PlayPreset(n) => raw.play_preset(n).await,
    }
}
