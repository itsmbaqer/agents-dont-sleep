//! Daily activity totals: one JSON file per local day in `~/.agents-dont-sleep/stats/`, kept 30
//! days. Fed by the main loop every tick.
//! ponytail: only counts while the app runs; agent time adds up overlapping sessions.
use crate::agents::Session;
use crate::settings::{data_dir, write_atomic};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

const KEEP_DAYS: i64 = 30;
/// Longest gap between ticks still counted as activity (the machine may have slept).
const MAX_STEP_SECS: u64 = 10;

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Day {
    /// Local date, "2026-09-23".
    pub date: String,
    /// Working seconds per agent id ("claude", or a process name for agents without hooks).
    pub agent_secs: BTreeMap<String, u64>,
    pub held_secs: u64,
    pub turns: u32,
    pub tools: u32,
    pub errors: u32,
    /// Battery percentage points used while kept awake on battery.
    pub battery_used: u32,
    pub sessions: Vec<SessionDay>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct SessionDay {
    pub key: String,
    pub agent: String,
    pub name: String,
    pub project: String,
    pub first_seen: u64,
    pub last_seen: u64,
    pub working_secs: u64,
    pub turns: u32,
    pub tools: u32,
    pub errors: u32,
}

pub fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn path(date: &str) -> std::path::PathBuf {
    data_dir().join("stats").join(format!("{date}.json"))
}

fn read(date: &str) -> Day {
    std::fs::read(path(date))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| Day { date: date.into(), ..Default::default() })
}

/// The last `n` days, oldest first; days without a file are empty.
pub fn days(n: i64) -> Vec<Day> {
    let today = chrono::Local::now().date_naive();
    (0..n).rev().map(|i| read(&(today - chrono::Duration::days(i)).format("%Y-%m-%d").to_string())).collect()
}

/// Deletes day files older than 30 days.
pub fn prune() {
    let cutoff = (chrono::Local::now().date_naive() - chrono::Duration::days(KEEP_DAYS)).format("%Y-%m-%d").to_string();
    for e in std::fs::read_dir(data_dir().join("stats")).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        // "YYYY-MM-DD.json" sorts by date as text.
        if name.len() == 15 && name.ends_with(".json") && name[..10] < *cutoff {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

pub struct Tracker {
    day: Day,
    started_at: u64,
    last_tick: Option<u64>,
    last_flush: u64,
    /// Last seen (turns, tools, errors) per session, to count only what happened since.
    counts: HashMap<String, (u32, u32, u32)>,
    last_battery: Option<u8>,
    /// Off in tests, so they never touch the real stats folder.
    persist: bool,
}

impl Tracker {
    pub fn load(now: u64) -> Self {
        Tracker {
            day: read(&today()),
            started_at: now,
            last_tick: None,
            last_flush: now,
            counts: HashMap::new(),
            last_battery: None,
            persist: true,
        }
    }

    pub fn day(&self) -> &Day {
        &self.day
    }

    /// Adds one loop tick. `date` is today's local date (passed in so day rollover is tested).
    #[allow(clippy::too_many_arguments)]
    pub fn tick(
        &mut self,
        now: u64,
        date: &str,
        sessions: &[Session],
        process_agents: &[String],
        held: bool,
        battery: Option<u8>,
        on_ac: bool,
    ) {
        if self.day.date != date {
            self.flush(now);
            self.day = Day { date: date.into(), ..Default::default() };
        }
        let dt = self.last_tick.map_or(0, |t| now.saturating_sub(t).min(MAX_STEP_SECS));
        self.last_tick = Some(now);

        for x in sessions.iter().filter(|x| !x.dismissed) {
            let key = format!("{}__{}", x.agent, x.id);
            // Sessions that predate the app start from their current counters, so a restart
            // doesn't count the same tool calls twice.
            let base = if x.started >= self.started_at { (0, 0, 0) } else { (x.turns, x.tools, x.errors) };
            let (t0, u0, e0) = *self.counts.entry(key.clone()).or_insert(base);
            let (dturns, dtools, derrors) =
                (x.turns.saturating_sub(t0), x.tools.saturating_sub(u0), x.errors.saturating_sub(e0));
            self.counts.insert(key.clone(), (x.turns.max(t0), x.tools.max(u0), x.errors.max(e0)));
            let active = if x.active() { dt } else { 0 };
            self.day.turns += dturns;
            self.day.tools += dtools;
            self.day.errors += derrors;
            *self.day.agent_secs.entry(x.agent.clone()).or_default() += active;

            let entry = match self.day.sessions.iter_mut().position(|s| s.key == key) {
                Some(i) => &mut self.day.sessions[i],
                None => {
                    self.day.sessions.push(SessionDay {
                        key: key.clone(),
                        agent: x.agent.clone(),
                        name: x.name.clone(),
                        project: x.project.clone(),
                        first_seen: now,
                        ..Default::default()
                    });
                    self.day.sessions.last_mut().expect("just pushed")
                }
            };
            entry.last_seen = now;
            entry.working_secs += active;
            entry.turns += dturns;
            entry.tools += dtools;
            entry.errors += derrors;
            if !x.project.is_empty() {
                entry.project = x.project.clone();
            }
        }
        for p in process_agents {
            *self.day.agent_secs.entry(p.to_lowercase()).or_default() += dt;
        }
        self.day.agent_secs.retain(|_, s| *s > 0);

        if held {
            self.day.held_secs += dt;
        }
        let draining = held && !on_ac;
        if let (true, Some(prev), Some(b)) = (draining, self.last_battery, battery) {
            self.day.battery_used += u32::from(prev.saturating_sub(b));
        }
        self.last_battery = if draining { battery } else { None };

        if now.saturating_sub(self.last_flush) >= 60 {
            self.flush(now);
        }
    }

    pub fn flush(&mut self, now: u64) {
        self.last_flush = now;
        if !self.persist {
            return;
        }
        if let Ok(json) = serde_json::to_vec(&self.day) {
            let _ = write_atomic(&path(&self.day.date), &json);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(state: &str, tools: u32, turns: u32, started: u64) -> Session {
        Session {
            agent: "claude".into(),
            id: "a".into(),
            name: "Claude Code".into(),
            project: "api".into(),
            state: state.into(),
            tools,
            turns,
            started,
            ..Default::default()
        }
    }

    fn tracker(now: u64) -> Tracker {
        Tracker {
            day: Day { date: "2026-09-23".into(), ..Default::default() },
            started_at: now,
            last_tick: None,
            last_flush: 0,
            counts: HashMap::new(),
            last_battery: None,
            persist: false,
        }
    }

    #[test]
    fn counts_work_and_deltas() {
        let mut t = tracker(1000);
        let d = "2026-09-23";
        // Started before the app: its 5 earlier tool calls aren't counted.
        t.tick(1000, d, &[session("working", 5, 1, 900)], &[], true, Some(80), false);
        t.tick(1002, d, &[session("working", 7, 1, 900)], &[], true, Some(80), false);
        t.tick(1004, d, &[session("idle", 7, 2, 900)], &[], true, Some(79), false);
        let day = t.day();
        assert_eq!((day.tools, day.turns, day.held_secs, day.battery_used), (2, 1, 4, 1));
        assert_eq!(day.agent_secs["claude"], 2); // the idle tick adds nothing
        assert_eq!(day.sessions.len(), 1);
        assert_eq!((day.sessions[0].working_secs, day.sessions[0].tools, day.sessions[0].first_seen), (2, 2, 1000));
    }

    #[test]
    fn new_sessions_count_from_zero() {
        let mut t = tracker(1000);
        t.tick(1010, "2026-09-23", &[session("working", 3, 0, 1005)], &[], false, None, true);
        assert_eq!(t.day().tools, 3);
    }

    #[test]
    fn gaps_are_capped_and_days_roll_over() {
        let mut t = tracker(1000);
        t.tick(1000, "2026-09-23", &[session("working", 0, 0, 1000)], &["aider".into()], true, None, true);
        t.tick(4600, "2026-09-23", &[session("working", 0, 0, 1000)], &["aider".into()], true, None, true); // slept an hour
        assert_eq!((t.day().held_secs, t.day().agent_secs["aider"]), (MAX_STEP_SECS, MAX_STEP_SECS));
        t.tick(4602, "2026-09-24", &[session("working", 0, 0, 1000)], &[], true, None, true);
        assert_eq!((t.day().date.as_str(), t.day().held_secs), ("2026-09-24", 2));
    }
}
