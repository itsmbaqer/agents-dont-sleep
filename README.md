# Agents Don't Sleep

A macOS menu-bar app that keeps your Mac awake, lid shut and in your bag, while Claude Code, Codex and other coding agents are working, then lets it sleep when they finish. It's a Tauri v2 + React clone of [Hold My Lid](https://holdmylid.app).

## How it works

- **Lid-closed awake** uses the kernel `SleepDisabled` flag (`pmset -a disablesleep 1`). A normal power assertion (`caffeinate`) can't survive a lid close. The flag needs root, so **Settings → General → Grant…** installs `/etc/sudoers.d/agents-dont-sleep` once. That rule allows exactly `pmset -a disablesleep 0|1` and nothing else. Without it, the app still holds a `caffeinate` assertion, which only covers the lid-open case.
- **Agents report their own state.** Each integration calls `~/.agents-dont-sleep/hook.sh <agent> <working|waiting|idle|end>`. The hook writes one small file per session to `~/.agents-dont-sleep/sessions/`, and the app polls that folder every 2 s. No server or ports are involved, and the hook never reads prompts, code or output.
- **Safety:**
  - Sleep is re-enabled when the app starts.
  - A detached watchdog re-enables it if the app ever dies, even from SIGKILL.
  - A thermal limit always applies, using `NSProcessInfo.thermalState`.
  - Battery cut-off, plugged-in-only and Low Power Mode settings are available.
  - The screen locks on lid close while the Mac is being held awake.

| Agent | Integration |
|---|---|
| Claude Code | hooks in `~/.claude/settings.json` |
| Codex | `~/.codex/hooks.json`, plus `[features] hooks = true` in `config.toml`. Run `/hooks` once to trust them. |
| Gemini CLI | hooks in `~/.gemini/settings.json` |
| Cursor | `~/.cursor/hooks.json` (observe-only events, so it never answers a permission prompt) |
| Copilot CLI | `~/.copilot/hooks/agents-dont-sleep.json` |
| OpenCode 1.x | plugin `~/.config/opencode/plugins/agents-dont-sleep.js` |
| Pi | extension `~/.pi/agent/extensions/agents-dont-sleep.ts` |
| Hermes | managed block in `~/.hermes/config.yaml`, entries in the allowlist, and a gateway hook |
| aider, goose, cline, conductor, … | process detection (the list is editable) |

Config edits only ever add or remove entries that contain `.agents-dont-sleep/hook.sh`. A `.bak` is written before each change, and files that can't be parsed are left untouched.

## Develop

```sh
pnpm install
pnpm tauri dev                      # menu-bar app + settings window
cd src-tauri && cargo test          # decide rules, parsers, config merge round-trips
pnpm tauri build                    # → src-tauri/target/release/bundle/{macos,dmg}
```

- **Code layout:** `src-tauri/src/` holds `lib.rs` (loop, tray, commands), `decide.rs` (pure hold/release rules), `power.rs` (pmset, sudoers, battery, lid, display), `agents.rs` (integrations and session scan), `settings.rs` and `license.rs`. The React settings window lives in `src/`.
- **Licensing** is off until you set `POLAR_ORG_ID` in `src-tauri/src/license.rs`. Once set, it gives a 7-day trial and uses Polar license keys, with the 3-Mac limit configured in Polar.
- **Dev builds** don't register the login item, and notifications show up under Terminal's name until the app is bundled.

## Verify on a real Mac

1. In Settings → General, click Grant… and check that `sudo -n -l /usr/bin/pmset -a disablesleep 1` exits 0.
2. Connect Claude Code, then run a long task. `pmset -g | grep SleepDisabled` should show `1` while it runs and `0` after it finishes.
3. On battery, start a 5-minute task, close the lid, wait, then open it. `pmset -g log | grep -E " Sleep | Wake "` should show no sleep while the task ran, and a sleep right after it finished.
4. `kill -9` the app while it's holding the Mac awake. `SleepDisabled` should return to `0` within 3 s.
