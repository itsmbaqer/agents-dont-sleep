# agents-dont-sleep

## 0.4.0

### Minor Changes

- [#11](https://github.com/itsmbaqer/agents-dont-sleep/pull/11) [`b9eda56`](https://github.com/itsmbaqer/agents-dont-sleep/commit/b9eda56f72f63e0059c59d1ee3a6748491e2604b) Thanks [@itsmbaqer](https://github.com/itsmbaqer)! - Data Saver for hotspots (macOS). A new **Data Saver · agents only** switch in the tray blocks every outgoing connection except DNS, your local network and an editable list of agent hosts (Claude Code, Codex, GitHub, npm and PyPI by default), so macOS updates, the App Store and app updaters can't use up your data while the lid is closed. It also turns off macOS and App Store automatic updates and puts them back afterwards.
  
  The first time, it asks for your password once. It installs a root-owned helper and a sudoers rule that allows exactly switching it on and off; Settings → General → Data Saver → Revoke removes both. It uses a pf anchor of its own and never edits `/etc/pf.conf`. It turns off when you quit or the app crashes. macOS has no per-app firewall without a signed Network Extension, so it allows by destination: other apps that talk to an allowed host still get through.

## 0.3.0

### Minor Changes

- [#9](https://github.com/itsmbaqer/agents-dont-sleep/pull/9) [`6a4455f`](https://github.com/itsmbaqer/agents-dont-sleep/commit/6a4455ffe0b0c0bd993245eb8412b3dc389093be) Thanks [@itsmbaqer](https://github.com/itsmbaqer)! - Usage limits in the tray. The menu now shows how much of your Claude Code and Codex limits is left, with a section for each: the 5-hour window and the weekly window, each with a reset countdown. The text beside the tray icon shows the 5-hour window left per provider, e.g. `C 46% · X 100%`. It follows the tray label setting and is hidden when that is set to Off. A header shows ⚠ when that provider has under 20% left or can't be read.
  
  The app reads the login each tool already stores, from the macOS Keychain or `~/.claude/.credentials.json` for Claude Code and from `~/.codex/auth.json` for Codex. It asks each provider's usage endpoint every 5 minutes using the system `curl`, so the app ships no new network code. Tokens are passed to curl on stdin, so they never show in the process list. A provider that isn't signed in doesn't appear. On macOS, the first read may show a Keychain prompt; choose Always Allow.
  
  These endpoints are not public APIs and can change without notice. When a read fails, the menu says why (sign in again, run claude to refresh an expired token, offline) instead of showing numbers. After a rate limit it keeps the last reading and waits 15 minutes before asking again.

## 0.2.0

### Minor Changes

- [#4](https://github.com/itsmbaqer/agents-dont-sleep/pull/4) [`1c22485`](https://github.com/itsmbaqer/agents-dont-sleep/commit/1c22485f0d0507c3b8a2cb5f70717554c0e2d480) Thanks [@itsmbaqer](https://github.com/itsmbaqer)! - Tray redesign and agent monitoring. Sessions are grouped by what needs attention, with live turn timers, the current tool, and a submenu per session: open the project folder, show its terminal app, or stop counting it. The tray icon shows when an agent needs you.
  
  New alerts tell you when an agent needs you, looks stuck, finishes a long turn, or stops with an error. You can keep awake without an agent (30 min to until you stop it), use "Sleep when agents finish", and see an Activity tab with daily stats and a 7-day chart. The hooks now record tool names, model and counts, never your prompts or code.
