//! Per-session alerts: an agent waiting on you, a session gone quiet, a long turn finished, a
//! turn that failed. Pure, so the once-per-episode rules are tested.
use crate::agents::Session;
use crate::settings::Settings;
use crate::tray::{dur, error_label, key};
use std::collections::HashMap;

#[derive(Debug, PartialEq)]
pub struct Alert {
    pub title: String,
    pub body: String,
    /// A long turn finished; the app skips its own "Agents finished" banner for this tick.
    pub finished: bool,
}

/// What was already announced per session.
#[derive(Default)]
pub struct Memory {
    seen: HashMap<String, Seen>,
}

#[derive(Default)]
struct Seen {
    waiting_alerted: bool,
    /// `last_event` of the quiet stretch we already flagged.
    stuck_at: u64,
    turns: u32,
    error_kind: String,
}

pub fn due(sessions: &[Session], mem: &mut Memory, set: &Settings) -> Vec<Alert> {
    let mut out = vec![];
    mem.seen.retain(|k, _| sessions.iter().any(|x| key(x) == *k));
    for x in sessions.iter().filter(|x| !x.dismissed) {
        let where_ = if x.project.is_empty() { String::new() } else { format!("{}: ", x.project) };
        let fresh = !mem.seen.contains_key(&key(x));
        let seen = mem.seen.entry(key(x)).or_default();
        if fresh {
            // Don't replay turns and errors that happened before we started watching.
            seen.turns = x.turns;
            seen.error_kind = x.error_kind.clone();
        }

        if x.state == "waiting" {
            if set.alert_waiting && !seen.waiting_alerted && x.waiting_secs >= u64::from(set.alert_waiting_mins) * 60 {
                seen.waiting_alerted = true;
                let what = if x.tool.is_empty() {
                    "waiting for you".to_string()
                } else {
                    format!("waiting to run {}", x.tool)
                };
                out.push(Alert {
                    title: format!("{} needs you", x.name),
                    body: format!("{where_}{what}"),
                    finished: false,
                });
            }
        } else {
            seen.waiting_alerted = false;
        }

        let stuck = set.alert_stuck_mins > 0 && x.quiet_secs >= u64::from(set.alert_stuck_mins) * 60;
        if x.state == "working" && stuck && seen.stuck_at != x.last_event {
            seen.stuck_at = x.last_event;
            let tool = if x.tool.is_empty() { String::new() } else { format!(" (still in {})", x.tool) };
            out.push(Alert {
                title: format!("{} looks stuck", x.name),
                body: format!("{where_}no activity for {}{tool}", dur(x.quiet_secs)),
                finished: false,
            });
        }

        if !x.error_kind.is_empty() && x.error_kind != seen.error_kind && set.alert_errors {
            out.push(Alert {
                title: format!("{} stopped", x.name),
                body: format!("{where_}{}", error_label(&x.error_kind)),
                finished: false,
            });
        } else if x.turns > seen.turns
            && x.error_kind.is_empty()
            && set.alert_long_turn_mins > 0
            && x.last_turn_secs >= u64::from(set.alert_long_turn_mins) * 60
        {
            out.push(Alert {
                title: format!("{} finished", x.name),
                body: format!("{where_}done after {}", dur(x.last_turn_secs)),
                finished: true,
            });
        }
        seen.turns = x.turns;
        seen.error_kind = x.error_kind.clone();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(state: &str) -> Session {
        Session {
            agent: "claude".into(),
            id: "a".into(),
            name: "Claude Code".into(),
            project: "api".into(),
            state: state.into(),
            ..Default::default()
        }
    }

    #[test]
    fn needs_you_once_per_wait() {
        let set = Settings { alert_waiting_mins: 1, ..Default::default() };
        let mut mem = Memory::default();
        let mut x = session("waiting");
        x.tool = "Bash".into();
        x.waiting_secs = 30;
        assert!(due(&[x.clone()], &mut mem, &set).is_empty()); // under a minute
        x.waiting_secs = 61;
        let a = due(&[x.clone()], &mut mem, &set);
        assert_eq!(a[0].title, "Claude Code needs you");
        assert_eq!(a[0].body, "api: waiting to run Bash");
        assert!(due(&[x.clone()], &mut mem, &set).is_empty()); // once
        x.state = "working".into();
        due(&[x.clone()], &mut mem, &set);
        x.state = "waiting".into();
        assert_eq!(due(&[x], &mut mem, &set).len(), 1); // a new wait alerts again
    }

    #[test]
    fn stuck_once_per_quiet_stretch() {
        let set = Settings { alert_stuck_mins: 15, ..Default::default() };
        let mut mem = Memory::default();
        let mut x = session("working");
        x.tool = "Bash".into();
        x.last_event = 1000;
        x.quiet_secs = 16 * 60;
        let a = due(&[x.clone()], &mut mem, &set);
        assert_eq!(
            (a[0].title.as_str(), a[0].body.as_str()),
            ("Claude Code looks stuck", "api: no activity for 16m (still in Bash)")
        );
        assert!(due(&[x.clone()], &mut mem, &set).is_empty());
        x.last_event = 2000; // activity, then quiet again
        assert_eq!(due(&[x], &mut mem, &set).len(), 1);
    }

    #[test]
    fn long_turns_and_errors() {
        let set = Settings { alert_long_turn_mins: 5, ..Default::default() };
        let mut mem = Memory::default();
        let mut x = session("working");
        x.turns = 2;
        assert!(due(&[x.clone()], &mut mem, &set).is_empty()); // first sighting: no replay
        x.state = "idle".into();
        x.turns = 3;
        x.last_turn_secs = 60;
        assert!(due(&[x.clone()], &mut mem, &set).is_empty()); // short turn
        x.turns = 4;
        x.last_turn_secs = 14 * 60;
        let a = due(&[x.clone()], &mut mem, &set);
        assert_eq!((a[0].body.as_str(), a[0].finished), ("api: done after 14m", true));
        x.turns = 5;
        x.error_kind = "rate_limit".into();
        let a = due(&[x.clone()], &mut mem, &set);
        assert_eq!((a.len(), a[0].title.as_str(), a[0].body.as_str()), (1, "Claude Code stopped", "api: rate limited"));
        assert!(due(&[x], &mut mem, &set).is_empty());
    }

    #[test]
    fn dismissed_and_disabled_stay_quiet() {
        let mut x = session("waiting");
        x.waiting_secs = 600;
        x.dismissed = true;
        assert!(due(&[x.clone()], &mut Memory::default(), &Settings::default()).is_empty());
        x.dismissed = false;
        let off = Settings { alert_waiting: false, ..Default::default() };
        assert!(due(&[x], &mut Memory::default(), &off).is_empty());
    }
}
