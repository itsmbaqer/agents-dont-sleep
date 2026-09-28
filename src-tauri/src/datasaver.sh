#!/bin/sh
# Agents Don't Sleep: Data Saver helper. Installed root-owned at
# /Library/PrivilegedHelperTools/agents-dont-sleep-datasaver; sudo lets the user run exactly
# `on` and `off`. `on` takes the allowed hosts on stdin (never a path), one per line.
#
# on:  block every outgoing connection except DNS/DHCP/NTP/mDNS, the local network and the
#      allowed hosts, and turn off macOS and App Store automatic updates. Idempotent: the app
#      re-runs it every 5 minutes so rotating CDN addresses pile up in the table.
# off: put all of it back. Safe to run when already off.
PATH=/usr/bin:/bin:/usr/sbin:/sbin
export PATH
ANCHOR=com.apple/250.agents-dont-sleep
STATE=/Library/PrivilegedHelperTools/agents-dont-sleep-datasaver.state
SU=/Library/Preferences/com.apple.SoftwareUpdate
AS=/Library/Preferences/com.apple.commerce

# Hosts, IPs and CIDRs from stdin; comments dropped, nothing that could be read as an option.
filter() {
    sed 's/#.*//' | tr -s ' \t\r' '\n' | grep -E '^[A-Za-z0-9][A-Za-z0-9.:/-]*$'
}

# Last match wins and nothing is `quick`, so anchors after ours (a VPN kill switch) still get
# the final say. `block return` fails connections at once instead of letting them hang.
# `pass in` keeps replies to inbound connections (SSH over Tailscale, a LAN client) working.
rules() {
    cat <<'EOF'
table <allow> persist
block return out all
pass out proto { udp tcp } from any to any port { 53 67 68 123 5353 } keep state
pass out from any to { 10.0.0.0/8 172.16.0.0/12 192.168.0.0/16 169.254.0.0/16 100.64.0.0/10 224.0.0.0/4 255.255.255.255 fe80::/10 ff00::/8 fc00::/7 } keep state
pass out from any to <allow> keep state
pass in all keep state
pass on lo0 all
EOF
}

pref() {
    v=$(defaults read "$1" "$2" 2>/dev/null) || v="unset"
    echo "$1 $2 $v"
}

on() {
    if [ ! -f "$STATE" ]; then
        { pref "$SU" AutomaticDownload; pref "$SU" AutomaticallyInstallMacOSUpdates; pref "$AS" AutoUpdate; } >"$STATE.tmp" &&
            mv "$STATE.tmp" "$STATE" || exit 1
    fi
    defaults write "$SU" AutomaticDownload -bool false
    defaults write "$SU" AutomaticallyInstallMacOSUpdates -bool false
    defaults write "$AS" AutoUpdate -bool false
    # Take an enable reference only when pf is off (first run, or something else disabled it).
    if ! pfctl -s info 2>/dev/null | grep -q 'Status: Enabled'; then
        echo "token $(pfctl -E 2>&1 | sed -n 's/^Token : //p')" >>"$STATE"
    fi
    # Load only when missing: a reload would empty the table under running agents.
    if ! pfctl -a "$ANCHOR" -s rules 2>/dev/null | grep -q block; then
        rules | pfctl -a "$ANCHOR" -f - 2>/dev/null || { echo "pf rejected the rules" >&2; exit 1; }
    fi
    # One call per entry: a host that doesn't resolve right now doesn't drop the rest.
    filter | while read -r h; do
        pfctl -a "$ANCHOR" -t allow -T add "$h" >/dev/null 2>&1
    done
    pfctl -a "$ANCHOR" -s rules 2>/dev/null | grep -q block || { echo "pf rules not loaded" >&2; exit 1; }
}

off() {
    pfctl -a "$ANCHOR" -F rules >/dev/null 2>&1
    pfctl -a "$ANCHOR" -F Tables >/dev/null 2>&1
    [ -f "$STATE" ] || return 0
    while read -r a b c; do
        if [ "$a" = token ]; then
            [ -n "$b" ] && pfctl -X "$b" >/dev/null 2>&1
        elif [ "$c" = unset ]; then
            defaults delete "$a" "$b" 2>/dev/null
        elif [ "$c" = 1 ]; then
            defaults write "$a" "$b" -bool true
        else
            defaults write "$a" "$b" -bool false
        fi
    done <"$STATE"
    rm -f "$STATE"
}

case "${1:-}" in
on) on ;;
off) off ;;
filter) filter ;;
*)
    echo "usage: $0 on|off" >&2
    exit 2
    ;;
esac
