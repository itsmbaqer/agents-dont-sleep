//! 5-hour and weekly usage limits for Claude Code and Codex, from the same private endpoints
//! their own `/usage` and `/status` screens use. Undocumented: any failure shows as "—".
use crate::settings::{home, now};
use serde::Serialize;
use serde_json::Value;
use std::{
    io::Write,
    process::{Command, Stdio},
};

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Window {
    pub used_pct: f64,
    pub resets_at: u64,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Limit {
    pub name: &'static str,
    /// One letter for the text beside the tray icon.
    pub short: &'static str,
    pub five_h: Option<Window>,
    pub week: Option<Window>,
    pub err: Option<String>,
}

/// Last readings plus per-provider back-off after a 429.
#[derive(Default)]
pub struct Poller {
    last: Vec<Limit>,
    wait_until: Vec<(&'static str, u64)>,
}

impl Poller {
    /// Providers that are signed in; ones that aren't installed are left out.
    pub fn poll(&mut self) -> Vec<Limit> {
        let mut out = vec![];
        for (name, short, fetch) in
            [("Claude", "C", claude as fn() -> Option<Fetched>), ("Codex", "X", codex as fn() -> Option<Fetched>)]
        {
            let prev = self.last.iter().find(|l| l.name == name).cloned();
            if self.wait_until.iter().any(|&(n, t)| n == name && t > now()) {
                out.extend(prev);
                continue;
            }
            let Some(got) = fetch() else { continue };
            let mut l = Limit { name, short, five_h: None, week: None, err: None };
            match got {
                Ok((five_h, week)) => (l.five_h, l.week) = (five_h, week),
                Err(Fail::RateLimited) => {
                    self.wait_until.retain(|&(n, _)| n != name);
                    self.wait_until.push((name, now() + 15 * 60));
                    // Keep showing the last good numbers meanwhile.
                    if let Some(p) = prev {
                        (l.five_h, l.week) = (p.five_h, p.week);
                    }
                    l.err = Some("rate limited, retrying in 15m".into());
                }
                Err(Fail::Msg(m)) => l.err = Some(m),
            }
            out.push(l);
        }
        self.last = out.clone();
        out
    }
}

enum Fail {
    RateLimited,
    Msg(String),
}

type Fetched = Result<(Option<Window>, Option<Window>), Fail>;

fn claude() -> Option<Fetched> {
    let raw = if cfg!(target_os = "macos") {
        crate::power::output("/usr/bin/security", &["find-generic-password", "-s", "Claude Code-credentials", "-w"])
    } else {
        std::fs::read_to_string(home().join(".claude/.credentials.json")).unwrap_or_default()
    };
    let creds: Value = serde_json::from_str(raw.trim()).ok()?;
    let oauth = &creds["claudeAiOauth"];
    let token = oauth["accessToken"].as_str().filter(|t| !t.is_empty())?;
    // An expired token gets a 429 with an hour-long Retry-After, not a 401, so don't send it.
    // Claude Code renews it the next time it runs.
    if oauth["expiresAt"].as_f64().is_some_and(|ms| ms / 1000.0 < now() as f64) {
        return Some(Err(Fail::Msg("run claude to refresh".into())));
    }
    let headers = format!("Authorization: Bearer {token}\nanthropic-beta: oauth-2025-04-20\n");
    Some(get("https://api.anthropic.com/api/oauth/usage", &headers).and_then(|b| parse_claude(&b)))
}

fn codex() -> Option<Fetched> {
    let raw = std::fs::read_to_string(home().join(".codex/auth.json")).ok()?;
    let auth: Value = serde_json::from_str(&raw).ok()?;
    let token = auth["tokens"]["access_token"].as_str().filter(|t| !t.is_empty())?;
    let account = auth["tokens"]["account_id"].as_str().unwrap_or_default();
    let headers = format!("Authorization: Bearer {token}\nChatGPT-Account-Id: {account}\n");
    Some(get("https://chatgpt.com/backend-api/wham/usage", &headers).and_then(|b| parse_codex(&b)))
}

/// GET via the system curl (macOS, Windows 10+, Linux), so there's no TLS stack to ship.
/// Headers go through stdin so the token never shows up in `ps`.
fn get(url: &str, headers: &str) -> Result<String, Fail> {
    let mut cmd = Command::new("curl");
    cmd.args(["-sS", "--max-time", "15", "-H", "@-", "-w", "\n%{http_code}", url])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().map_err(|_| Fail::Msg("curl not found".into()))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(headers.as_bytes());
    }
    let out = child.wait_with_output().map_err(|e| Fail::Msg(e.to_string()))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let (body, code) = text.rsplit_once('\n').unwrap_or(("", &text));
    match code.trim().parse::<u16>().unwrap_or(0) {
        200..300 => Ok(body.to_string()),
        401 | 403 => Err(Fail::Msg("sign in again".into())),
        429 => Err(Fail::RateLimited),
        0 => Err(Fail::Msg("offline".into())),
        c => Err(Fail::Msg(format!("HTTP {c}"))),
    }
}

fn bad_json() -> Fail {
    Fail::Msg("unexpected response".into())
}

/// `{"five_hour":{"utilization":42.0,"resets_at":"2026-…Z"},"seven_day":{…}}`, 0–100.
/// A window is null until its first use.
fn parse_claude(body: &str) -> Fetched {
    let v: Value = serde_json::from_str(body).map_err(|_| bad_json())?;
    let win = |w: &Value| {
        Some(Window {
            used_pct: w["utilization"].as_f64()?,
            resets_at: w["resets_at"]
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map_or(0, |d| d.timestamp().max(0) as u64),
        })
    };
    Ok((win(&v["five_hour"]), win(&v["seven_day"])))
}

/// `{"rate_limit":{"primary_window":{"used_percent":25,"reset_at":1800001000},"secondary_window":{…}}}`:
/// primary is the 5-hour window, secondary the weekly one.
fn parse_codex(body: &str) -> Fetched {
    let v: Value = serde_json::from_str(body).map_err(|_| bad_json())?;
    let win = |w: &Value| {
        Some(Window {
            used_pct: w["used_percent"].as_f64()?,
            resets_at: w["reset_at"]
                .as_u64()
                .or_else(|| w["reset_after_seconds"].as_u64().map(|s| now() + s))
                .unwrap_or(0),
        })
    };
    let rl = &v["rate_limit"];
    Ok((win(&rl["primary_window"]), win(&rl["secondary_window"])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_both() {
        let Ok((five, week)) = parse_claude(
            r#"{"five_hour":{"utilization":42.5,"resets_at":"2030-01-01T00:00:00.123+00:00"},"seven_day":null}"#,
        ) else {
            panic!()
        };
        assert_eq!(five, Some(Window { used_pct: 42.5, resets_at: 1_893_456_000 }));
        assert_eq!(week, None);

        let Ok((five, week)) = parse_codex(
            r#"{"rate_limit":{"primary_window":{"used_percent":25,"reset_at":1800001000},
                "secondary_window":{"used_percent":80,"reset_after_seconds":60}}}"#,
        ) else {
            panic!()
        };
        assert_eq!(five, Some(Window { used_pct: 25.0, resets_at: 1_800_001_000 }));
        assert!(week.is_some_and(|w| w.used_pct == 80.0 && w.resets_at >= now() + 59));
        assert!(parse_codex("<html>").is_err());
    }
}
