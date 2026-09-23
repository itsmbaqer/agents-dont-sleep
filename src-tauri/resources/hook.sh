#!/bin/sh
# Agents Don't Sleep hook: hook.sh <agent> <working|waiting|idle|end> [session-id] [cwd]
# Records the agent's state as a small file the menu-bar app polls. Installed by the app;
# remove it from Settings → Agents. Never blocks and never fails the agent.
agent="$1" state="$2" sid="$3" cwd="$4"
if [ -z "$sid" ]; then
  input=$(cat 2>/dev/null)
  get() { printf '%s' "$input" | /usr/bin/plutil -extract "$1" raw -o - - 2>/dev/null; }
  sid=$(get session_id || get sessionId || get conversation_id)
  cwd=$(get cwd)
fi
sid=$(printf '%s' "${sid:-pid-$PPID}" | tr -c 'A-Za-z0-9._-' '_')
# The agent's pid, so the app can drop this session the moment that process exits.
# Skip a wrapping `sh -c` if the agent ran us through one.
pid=$PPID
case "$(ps -o comm= -p "$pid" 2>/dev/null)" in
  *sh) pid=$(ps -o ppid= -p "$pid" | tr -d ' ') ;;
esac
dir="$(dirname "$0")/sessions"
f="$dir/${agent}__${sid}"
if [ "$state" = end ]; then
  rm -f "$f"
else
  mkdir -p "$dir" && printf '%s\n%s\n%s\n' "$state" "$cwd" "$pid" > "$dir/.tmp.$$" && mv -f "$dir/.tmp.$$" "$f"
fi
# Agents that parse stdout get an explicit no-op.
case "$agent" in
  copilot|gemini) printf '{}' ;;
  cursor) printf '{"continue":true}' ;;
esac
exit 0
