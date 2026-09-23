# Security

## Reporting a vulnerability

Please report security issues privately through **GitHub → Security → Report a vulnerability** on this repository, not in a public issue. You'll get a reply within a few days.

## What the app changes on your system

Agents Don't Sleep only changes what it needs to keep a laptop awake with the lid closed, and it changes things back when agents finish, when you quit, and after a crash. A separate watchdog process restores the settings if the app is killed. The app itself also restores them the next time it starts.

### macOS

| Change | When | Undo by hand |
|---|---|---|
| `/etc/sudoers.d/agents-dont-sleep`, which lets your user run exactly `/usr/bin/pmset -a disablesleep 0` and `… 1` without a password. It's validated with `visudo` and owned by root with mode 0440. | Once, when you click **Grant…** (admin password prompt) | `sudo rm /etc/sudoers.d/agents-dont-sleep`, or **Settings → General → Revoke** |
| Kernel `SleepDisabled` flag on | Only while an agent is working | `sudo pmset -a disablesleep 0` |
| A `caffeinate -i` assertion | Only while an agent is working | Quit the app |

### Windows

| Change | When | Undo by hand |
|---|---|---|
| Active power plan: lid close action set to "Do nothing" (plugged in and on battery), and the battery sleep timeout set to "Never". The original values are saved to `%USERPROFILE%\.agents-dont-sleep\restore.json` first. | Only while an agent is working | Control Panel → Power Options → Choose what closing the lid does |
| A "system required" power request (visible in `powercfg /requests`) | Only while an agent is working | Quit the app |

No administrator rights are used.

### Linux

| Change | When | Undo by hand |
|---|---|---|
| A logind block inhibitor for `sleep:idle:handle-lid-switch` (visible in `systemd-inhibit --list`) | Only while an agent is working | Quit the app (the inhibitor exits with it) |
| KDE Plasma only: `LidAction=0` in `~/.config/powerdevilrc` for the AC, Battery and LowBattery groups. The originals are saved to `~/.agents-dont-sleep/restore.json`. | Only while an agent is working | System Settings → Power Management |

No root is used.

### Agent configs (all platforms)

**Connect** adds hook entries to that agent's own config, such as `~/.claude/settings.json` or `~/.codex/hooks.json`. The entries call `~/.agents-dont-sleep/bin/adshook`.
- Only entries that mention `adshook` are ever added or removed.
- A `.bak` copy is written before every change.
- A file that isn't valid JSON is left untouched.

**Hermes:**
- The app adds a marked block to `~/.hermes/config.yaml`.
- It pre-approves its own commands in `~/.hermes/shell-hooks-allowlist.json`.
- It installs a gateway hook in `~/.hermes/hooks/agents-dont-sleep/`.

**Disconnect**, or **Settings → General → Remove all**, reverses all of this.

## Privacy

- The hook helper parses each hook payload only to pick out the session id and working folder. Nothing else from it (prompts, code, tool output) is kept.
- Everything it records stays in `~/.agents-dont-sleep/`.
- The app makes no network requests and has no telemetry.
