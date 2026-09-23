// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // `--watchdog <pid>`: restore power settings once the app process exits (see power::spawn_watchdog).
    if let (Some("--watchdog"), Some(pid)) = (args.get(1).map(String::as_str), args.get(2).and_then(|p| p.parse().ok()))
    {
        agents_dont_sleep_lib::watchdog(pid);
        return;
    }
    agents_dont_sleep_lib::run()
}
