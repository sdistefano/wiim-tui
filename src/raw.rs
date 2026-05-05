//! Raw HTTP client for LinkPlay endpoints that `wiim_api` 0.1 doesn't
//! expose. Two right now: source switching and preset playback. Self-
//! signed certs accepted (WiiM ships its own CA).

use std::time::Duration;

use color_eyre::eyre::{Result, eyre};
use reqwest::Client;

#[derive(Debug, Clone)]
pub struct RawClient {
    base_url: String,
    http: Client,
}

impl RawClient {
    pub fn new(host: &str) -> Self {
        let base_url = if host.starts_with("http") {
            host.to_string()
        } else {
            format!("https://{host}")
        };
        let http = Client::builder()
            .danger_accept_invalid_certs(true)
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .build()
            .expect("reqwest builder always succeeds with valid args");
        Self { base_url, http }
    }

    /// Fire a raw `httpapi.asp?command=...` and return the body. Most
    /// LinkPlay command endpoints reply with literal `OK` on success.
    async fn cmd(&self, command: &str) -> Result<String> {
        let url = format!("{}/httpapi.asp?command={command}", self.base_url);
        let body = self.http.get(&url).send().await?.text().await?;
        Ok(body)
    }

    /// `setPlayerCmd:switchmode:<arg>`. arg examples: `wifi`, `bluetooth`,
    /// `line-in`, `line-in2`, `optical`, `co-axial`, `udisk`. The device
    /// silently accepts unsupported sources on some firmwares (returns
    /// "OK" but doesn't switch); we don't try to detect that.
    pub async fn switch_mode(&self, arg: &str) -> Result<()> {
        let body = self.cmd(&format!("setPlayerCmd:switchmode:{arg}")).await?;
        ensure_ok(&body)
    }

    /// `MCUKeyShortClick:N` — plays preset slot N (1-based). Slots are
    /// configured via the official WiiM app; this only fires them.
    pub async fn play_preset(&self, n: u8) -> Result<()> {
        let body = self.cmd(&format!("MCUKeyShortClick:{n}")).await?;
        ensure_ok(&body)
    }
}

fn ensure_ok(body: &str) -> Result<()> {
    let trimmed = body.trim();
    // LinkPlay returns plain "OK" for command success. Anything else
    // (including empty body, JSON error blob, or "fail") is suspect.
    if trimmed.eq_ignore_ascii_case("ok") {
        Ok(())
    } else {
        Err(eyre!("device returned non-OK: {trimmed}"))
    }
}
