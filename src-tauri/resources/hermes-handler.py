# Agents Don't Sleep: forwards Hermes gateway events to the adshook helper. Safe to delete.
import subprocess

HOOK = __HOOK__
STATES = {
    "agent:start": "working",
    "agent:step": "working",
    "agent:end": "idle",
    "session:end": "end",
    "session:reset": "end",
}


def handle(event_type, context):
    state = STATES.get(event_type)
    if not state:
        return
    sid = str((context or {}).get("session_id") or "gateway")
    subprocess.run(
        [HOOK, "hermes", state, sid, "", f"--event={event_type}"],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        timeout=5,
        check=False,
    )
