// Agents Don't Sleep: reports OpenCode session state to the menu-bar app. Safe to delete;
// reinstall from Settings → Agents.
const HOOK = "__HOOK__";

export const AgentsDontSleep = async ({ $, directory }) => ({
  event: async ({ event: e }) => {
    const p = e.properties ?? {};
    const id = p.sessionID ?? p.info?.id;
    let state;
    if (e.type === "session.status") state = p.status?.type === "idle" ? "idle" : "working";
    else if (e.type === "session.idle") state = "idle";
    else if (e.type === "permission.asked" || e.type === "permission.updated") state = "waiting";
    else if (e.type === "session.deleted") state = "end";
    if (state && id) await $`${HOOK} opencode ${state} ${id} ${directory ?? ""}`.nothrow().quiet();
  },
});
