# wiim-tui

Keyboard-driven terminal UI for [WiiM](https://www.wiimhome.com/) (LinkPlay) audio streamers on the local network.

Think `ncmpcpp`, but for WiiM: browse devices found via mDNS, control playback, manage multiroom groups, and tweak EQ —
all from the keyboard, over SSH if you want.

## Status

**Phase 3 — sources, presets, view switching.** NowPlaying view (title, artist, album, progress, volume, source);
press `s` to cycle sources, `1`-`9` to fire radio presets configured in the WiiM app, `Tab` / `n` / `q` to switch
between NowPlaying and Queue views. Polls the device every second; commands run optimistically.

mDNS auto-discovery (Phase 2) was skipped — set the device IP in `~/.config/wiim-tui/config.toml` instead.

## Quickstart

```sh
cargo run --release -- --device 192.168.1.42
```

Or set a default device once and run with no flags:

```sh
mkdir -p ~/.config/wiim-tui
cat > ~/.config/wiim-tui/config.toml <<'EOF'
device = "192.168.1.42"
EOF
cargo run --release
```

`--device` always overrides the config file. `Ctrl+C` quits cleanly (no stuck terminal).

Keys:

| Key            | Action                       |
| -------------- | ---------------------------- |
| `Space`        | Play / pause                 |
| `+` / `-`      | Volume ± 5%                  |
| `=`            | Volume +1% (fine)            |
| `m`            | Mute toggle                  |
| `>` / `<`      | Next / previous              |
| `s`            | Cycle audio source           |
| `1`–`9`        | Play radio preset N          |
| `Tab` / `BTab` | Cycle views                  |
| `n`            | NowPlaying view              |
| `q`            | Queue view                   |
| `Esc`          | Dismiss error                |
| `Q` / `Ctrl+C` | Quit                         |

Logs are written to `~/.cache/wiim-tui/log.txt`. Set `RUST_LOG=debug` for more verbosity.

## Targets

Linux x86_64 only. macOS / Windows are not supported and not on the roadmap.

## Threat model

WiiM devices use self-signed HTTPS certificates. From Phase 1 onward, wiim-tui disables certificate verification when
talking to them — appropriate for a trusted home LAN, **not** appropriate over the public internet. Don't expose your
WiiM devices outside your LAN, and don't use this tool over an untrusted network.

## Arch Linux / AUR

A `PKGBUILD` lives in [`pkg/`](pkg/PKGBUILD). To build and install locally from a release tarball:

```sh
git tag -l                      # confirm v0.1.0 exists
git archive --format=tar.gz --prefix=wiim-tui-0.1.0/ v0.1.0 -o pkg/wiim-tui-0.1.0.tar.gz
cd pkg && makepkg -si
```

## License

MIT — see [LICENSE](LICENSE).
