---
"agents-dont-sleep": minor
---

Usage limits in the tray. The menu now shows how much of your Claude Code and Codex limits is left, with a section for each: the 5-hour window and the weekly window, each with a reset countdown. The text beside the tray icon shows the 5-hour window left per provider, e.g. `C 46% · X 100%`. It follows the tray label setting and is hidden when that is set to Off. A header shows ⚠ when that provider has under 20% left or can't be read.

The app reads the login each tool already stores, from the macOS Keychain or `~/.claude/.credentials.json` for Claude Code and from `~/.codex/auth.json` for Codex. It asks each provider's usage endpoint every 5 minutes using the system `curl`, so the app ships no new network code. Tokens are passed to curl on stdin, so they never show in the process list. A provider that isn't signed in doesn't appear. On macOS, the first read may show a Keychain prompt; choose Always Allow.

These endpoints are not public APIs and can change without notice. When a read fails, the menu says why (sign in again, run claude to refresh an expired token, offline) instead of showing numbers. After a rate limit it keeps the last reading and waits 15 minutes before asking again.
