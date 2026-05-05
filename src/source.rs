//! Audio source / input mode.
//!
//! LinkPlay reports the active source as a numeric string in
//! `getPlayerStatus.mode`. We parse it into a stable enum, since the
//! number-to-source mapping is the same across firmwares but the labels
//! aren't standardised.
//!
//! Setting a source uses a different vocabulary
//! (`setPlayerCmd:switchmode:<arg>`), so each variant carries both a
//! human label and the switch arg.

use wiim_api::{NowPlaying, PlayerStatus};

/// Combined per-poll snapshot. `NowPlaying` covers track + transport;
/// `Source` is the active input. The poll task assembles this from a
/// single `getPlayerStatus` + `getMetaInfo` round-trip.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub now_playing: NowPlaying,
    pub source: Source,
}

/// Sources we recognise. `Network` collapses the streaming stack
/// (Wi-Fi/AirPlay/DLNA/Spotify Connect/Tidal Connect/...) since they
/// share the same physical input and switching among them is driven by
/// the source app, not the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Network,
    Bluetooth,
    LineIn,
    LineIn2,
    Optical,
    Coaxial,
    Usb,
    Unknown,
}

impl Source {
    /// Parse from `PlayerStatus.mode` numeric string. Codes derived from
    /// the LinkPlay HTTP API reverse-engineering notes; gaps map to
    /// `Unknown` rather than failing parse.
    pub fn from_mode_str(s: &str) -> Self {
        match s.trim() {
            "1" | "2" | "10" | "20" | "31" | "36" | "37" => Self::Network,
            "11" | "16" | "51" => Self::Usb,
            "40" => Self::LineIn,
            "47" => Self::LineIn2,
            "41" => Self::Bluetooth,
            "43" => Self::Optical,
            "44" => Self::Coaxial,
            _ => Self::Unknown,
        }
    }

    pub fn from_status(status: &PlayerStatus) -> Self {
        Self::from_mode_str(&status.mode)
    }

    /// Short label shown in the UI.
    pub fn label(self) -> &'static str {
        match self {
            Self::Network => "Network",
            Self::Bluetooth => "Bluetooth",
            Self::LineIn => "Line In",
            Self::LineIn2 => "Line In 2",
            Self::Optical => "Optical",
            Self::Coaxial => "Coaxial",
            Self::Usb => "USB",
            Self::Unknown => "—",
        }
    }

    /// Argument for `setPlayerCmd:switchmode:<arg>`. `None` for sources
    /// we can't switch back to (e.g. Spotify Connect handed-off
    /// sessions; the source app must initiate).
    pub fn switch_arg(self) -> Option<&'static str> {
        match self {
            Self::Network => Some("wifi"),
            Self::Bluetooth => Some("bluetooth"),
            Self::LineIn => Some("line-in"),
            Self::LineIn2 => Some("line-in2"),
            Self::Optical => Some("optical"),
            Self::Coaxial => Some("co-axial"),
            Self::Usb => Some("udisk"),
            Self::Unknown => None,
        }
    }
}

/// The order `s` cycles through. Limited to inputs the user can plausibly
/// switch to; Coaxial/USB/LineIn2 are present on bigger units only and
/// are appended at the end so they're reachable but don't dominate the
/// rotation on a Mini.
const CYCLE: &[Source] = &[
    Source::Network,
    Source::Bluetooth,
    Source::LineIn,
    Source::Optical,
    Source::Coaxial,
    Source::Usb,
    Source::LineIn2,
];

/// Pick the next source after `current`. Wraps. If `current` isn't in
/// the cycle (e.g. `Unknown`), starts at the head.
pub fn next_in_cycle(current: Source) -> Source {
    let idx = CYCLE
        .iter()
        .position(|s| *s == current)
        .unwrap_or(usize::MAX);
    let next = idx.wrapping_add(1) % CYCLE.len();
    CYCLE[next]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_modes() {
        assert_eq!(Source::from_mode_str("10"), Source::Network);
        assert_eq!(Source::from_mode_str("31"), Source::Network); // Spotify Connect
        assert_eq!(Source::from_mode_str("36"), Source::Network); // Tidal Connect
        assert_eq!(Source::from_mode_str("41"), Source::Bluetooth);
        assert_eq!(Source::from_mode_str("40"), Source::LineIn);
        assert_eq!(Source::from_mode_str("43"), Source::Optical);
    }

    #[test]
    fn unknown_modes_dont_panic() {
        assert_eq!(Source::from_mode_str("99"), Source::Unknown);
        assert_eq!(Source::from_mode_str(""), Source::Unknown);
        assert_eq!(Source::from_mode_str("garbage"), Source::Unknown);
    }

    #[test]
    fn cycle_advances_and_wraps() {
        assert_eq!(next_in_cycle(Source::Network), Source::Bluetooth);
        assert_eq!(next_in_cycle(Source::Bluetooth), Source::LineIn);
        assert_eq!(next_in_cycle(Source::LineIn2), Source::Network); // wrap
    }

    #[test]
    fn unknown_source_starts_cycle_at_head() {
        // `Unknown` shouldn't strand the user — `s` should land them on
        // a known source.
        assert_eq!(next_in_cycle(Source::Unknown), Source::Network);
    }
}
