//! View rendering. The top-level `render` is a pure function over an
//! immutable `App` snapshot; the per-view bodies live below.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, Paragraph};
use wiim_api::{NowPlaying, PlayState};

use crate::app::{App, View};
use crate::source::Source;

pub fn render(app: &App, frame: &mut Frame) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Min(8),    // body (per view)
            Constraint::Length(1), // error/status line
            Constraint::Length(1), // keybind hints
        ])
        .split(area);

    render_header(app, frame, chunks[0]);
    match app.view {
        View::NowPlaying => render_now_playing(app, frame, chunks[1]),
        View::Queue => render_queue(app, frame, chunks[1]),
    }
    render_error(app, frame, chunks[2]);
    render_hints(app, frame, chunks[3]);
}

fn render_header(app: &App, frame: &mut Frame, area: Rect) {
    let view_label = match app.view {
        View::NowPlaying => "now playing",
        View::Queue => "queue",
    };
    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            "wiim-tui",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            view_label,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::DIM),
        ),
        Span::raw("  "),
        Span::styled(
            format!("device {}", app.device_ip),
            Style::default().fg(Color::DarkGray),
        ),
    ]))
    .alignment(Alignment::Center)
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(title, area);
}

fn render_now_playing(app: &App, frame: &mut Frame, area: Rect) {
    let Some(np) = app.now_playing.as_ref() else {
        let placeholder = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "Connecting…",
                Style::default().fg(Color::DarkGray),
            )),
        ])
        .alignment(Alignment::Center);
        frame.render_widget(placeholder, area);
        return;
    };

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // album
            Constraint::Length(1), // artist
            Constraint::Length(1), // title (bold)
            Constraint::Length(1), // spacer
            Constraint::Length(1), // progress label (time)
            Constraint::Length(1), // progress bar
            Constraint::Length(1), // spacer
            Constraint::Length(1), // volume label
            Constraint::Length(1), // volume bar
            Constraint::Length(1), // spacer
            Constraint::Length(1), // source line
            Constraint::Min(0),    // padding
        ])
        .split(centered(area, 70));

    frame.render_widget(
        line(np.album.as_deref().unwrap_or("—"), Color::Gray),
        rows[0],
    );
    frame.render_widget(
        line(np.artist.as_deref().unwrap_or("—"), Color::White),
        rows[1],
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            np.title.as_deref().unwrap_or("—").to_string(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))),
        rows[2],
    );

    frame.render_widget(progress_label(np), rows[4]);
    frame.render_widget(progress_gauge(np), rows[5]);

    frame.render_widget(volume_label(np), rows[7]);
    frame.render_widget(volume_gauge(np), rows[8]);

    frame.render_widget(source_line(app.source), rows[10]);
}

fn render_queue(app: &App, frame: &mut Frame, area: Rect) {
    // Phase 3 queue is intentionally minimal. The LinkPlay HTTP API
    // exposes per-track listing only for device-local sources (USB,
    // local playlists). On Spotify Connect / Tidal Connect / AirPlay
    // the queue lives in the source app; we'd return a list of one.
    // Until we plumb the queue endpoint we just surface what we know.
    let body = centered(area, 70);
    let lines: Vec<Line> = match app.now_playing.as_ref() {
        None => vec![Line::from(Span::styled(
            "Connecting…",
            Style::default().fg(Color::DarkGray),
        ))],
        Some(np) => {
            let title = np.title.as_deref().unwrap_or("—");
            let artist = np.artist.as_deref().unwrap_or("—");
            vec![
                Line::from(Span::styled(
                    "Current",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    format!("▶ {title}"),
                    Style::default().fg(Color::Cyan),
                )),
                Line::from(Span::styled(
                    format!("  {artist}"),
                    Style::default().fg(Color::White),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    format!("Source: {}", app.source.label()),
                    Style::default().fg(Color::Gray),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    queue_note(app.source),
                    Style::default().fg(Color::DarkGray),
                )),
            ]
        }
    };
    frame.render_widget(Paragraph::new(lines), body);
}

fn queue_note(source: Source) -> &'static str {
    match source {
        Source::Network => {
            "Streaming sources (Spotify Connect / Tidal Connect / AirPlay) keep the queue \
in the source app — it isn't exposed to the device."
        }
        Source::Bluetooth
        | Source::LineIn
        | Source::LineIn2
        | Source::Optical
        | Source::Coaxial => "Analog inputs don't have a queue.",
        Source::Usb => "USB queue listing is not yet wired up.",
        Source::Unknown => "",
    }
}

fn line(text: &str, fg: Color) -> Paragraph<'_> {
    Paragraph::new(Line::from(Span::styled(
        text.to_string(),
        Style::default().fg(fg),
    )))
}

fn progress_label(np: &NowPlaying) -> Paragraph<'_> {
    let pos = format_ms(np.position_ms);
    let dur = format_ms(np.duration_ms);
    let state = play_state_label(&np.state);
    Paragraph::new(Line::from(vec![
        Span::styled(state, Style::default().fg(Color::Yellow)),
        Span::raw("  "),
        Span::styled(
            format!("{pos} / {dur}"),
            Style::default().fg(Color::DarkGray),
        ),
    ]))
}

fn progress_gauge(np: &NowPlaying) -> Gauge<'_> {
    let ratio = if np.duration_ms == 0 {
        0.0
    } else {
        (np.position_ms as f64 / np.duration_ms as f64).clamp(0.0, 1.0)
    };
    Gauge::default()
        .gauge_style(Style::default().fg(Color::Cyan).bg(Color::Black))
        .ratio(ratio)
        .label("")
}

fn volume_label(np: &NowPlaying) -> Paragraph<'_> {
    let muted = if np.is_muted { " (muted)" } else { "" };
    Paragraph::new(Line::from(Span::styled(
        format!("Vol {}%{}", np.volume, muted),
        Style::default().fg(if np.is_muted {
            Color::DarkGray
        } else {
            Color::White
        }),
    )))
}

fn volume_gauge(np: &NowPlaying) -> Gauge<'_> {
    let ratio = (np.volume as f64 / 100.0).clamp(0.0, 1.0);
    let colour = if np.is_muted {
        Color::DarkGray
    } else {
        Color::Green
    };
    Gauge::default()
        .gauge_style(Style::default().fg(colour).bg(Color::Black))
        .ratio(ratio)
        .label("")
}

fn source_line(source: Source) -> Paragraph<'static> {
    Paragraph::new(Line::from(vec![
        Span::styled("Source:  ", Style::default().fg(Color::DarkGray)),
        Span::styled(source.label(), Style::default().fg(Color::White)),
        Span::raw("    "),
        Span::styled("(s to cycle)", Style::default().fg(Color::DarkGray)),
    ]))
}

fn render_error(app: &App, frame: &mut Frame, area: Rect) {
    let Some(err) = app.error.as_ref() else {
        return;
    };
    let p = Paragraph::new(Line::from(vec![
        Span::styled(" ! ", Style::default().bg(Color::Red).fg(Color::White)),
        Span::raw(" "),
        Span::styled(err.clone(), Style::default().fg(Color::Red)),
        Span::raw("  "),
        Span::styled("(Esc to dismiss)", Style::default().fg(Color::DarkGray)),
    ]));
    frame.render_widget(p, area);
}

fn render_hints(app: &App, frame: &mut Frame, area: Rect) {
    // Keep the hint line sized to the active view so the user only sees
    // keys that do something here.
    let hint = match app.view {
        View::NowPlaying => Paragraph::new(Line::from(vec![
            key(" Space "),
            Span::raw("play  "),
            key("+/-"),
            Span::raw(" vol  "),
            key("m"),
            Span::raw(" mute  "),
            key("</>"),
            Span::raw(" prev/next  "),
            key("s"),
            Span::raw(" source  "),
            key("1-9"),
            Span::raw(" preset  "),
            key("Tab"),
            Span::raw(" view  "),
            key("Q"),
            Span::raw(" quit"),
        ])),
        View::Queue => Paragraph::new(Line::from(vec![
            key(" n "),
            Span::raw("now playing  "),
            key("Tab"),
            Span::raw(" view  "),
            key("Q"),
            Span::raw(" quit"),
        ])),
    }
    .style(Style::default().bg(Color::Rgb(20, 20, 20)));
    frame.render_widget(hint, area);
}

fn key(label: &str) -> Span<'static> {
    Span::styled(label.to_string(), Style::default().fg(Color::Yellow))
}

fn centered(area: Rect, width: u16) -> Rect {
    let w = width.min(area.width);
    let x_pad = area.width.saturating_sub(w) / 2;
    Rect {
        x: area.x + x_pad,
        y: area.y,
        width: w,
        height: area.height,
    }
}

fn format_ms(ms: u64) -> String {
    let total = ms / 1000;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

fn play_state_label(s: &PlayState) -> &'static str {
    match s {
        PlayState::Playing => "▶ playing",
        PlayState::Paused => "‖ paused",
        PlayState::Stopped => "■ stopped",
        PlayState::Loading => "… loading",
    }
}
