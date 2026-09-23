// Agents Don't Sleep: reports OpenCode session activity (state and tool names only) to the
// menu-bar app. Safe to delete; reinstall from Settings → Agents.
const HOOK = __HOOK__;
const seen = new Set(); // tool calls already reported (parts update several times each)

export const AgentsDontSleep = async ({ $, directory }) => ({
  event: async ({ event: e }) => {
    const p = e.properties ?? {};
    let id = p.sessionID ?? p.info?.id;
    let state;
    let name = e.type;
    let tool = "";
    if (e.type === "session.status") state = p.status?.type === "idle" ? "idle" : "working";
    else if (e.type === "session.idle") state = "idle";
    else if (e.type === "permission.asked" || e.type === "permission.updated") state = "waiting";
    else if (e.type === "session.deleted") state = "end";
    else if (e.type === "message.part.updated" && p.part?.type === "tool") {
      const key = p.part.callID ?? p.part.id;
      if (!key || seen.has(key)) return;
      if (seen.size > 5000) seen.clear();
      seen.add(key);
      id = p.part.sessionID ?? id;
      state = "working";
      name = "tool";
      tool = String(p.part.tool ?? "");
    }
    if (state && id) {
      await $`${HOOK} opencode ${state} ${id} ${directory ?? ""} ${"--event=" + name} ${"--tool=" + tool}`
        .nothrow()
        .quiet();
    }
  },
});
