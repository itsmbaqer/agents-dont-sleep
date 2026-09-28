---
"agents-dont-sleep": minor
---

Data Saver for hotspots (macOS). A new **Data Saver · agents only** switch in the tray blocks every outgoing connection except DNS, your local network and an editable list of agent hosts (Claude Code, Codex, GitHub, npm and PyPI by default), so macOS updates, the App Store and app updaters can't use up your data while the lid is closed. It also turns off macOS and App Store automatic updates and puts them back afterwards.

The first time, it asks for your password once. It installs a root-owned helper and a sudoers rule that allows exactly switching it on and off; Settings → General → Data Saver → Revoke removes both. It uses a pf anchor of its own and never edits `/etc/pf.conf`. It turns off when you quit or the app crashes. macOS has no per-app firewall without a signed Network Extension, so it allows by destination: other apps that talk to an allowed host still get through.
