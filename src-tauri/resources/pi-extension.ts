// Agents Don't Sleep: reports pi's agent lifecycle (state and tool names only) to the menu-bar
// app. Safe to delete; reinstall from Settings → Agents.
const HOOK = __HOOK__;

export default function (pi: any) {
  const send = (state: string, ctx: any, event: string, tool = "") =>
    pi
      .exec(HOOK, [
        "pi",
        state,
        String(ctx?.sessionManager?.getSessionId?.() ?? `pid-${process.pid}`),
        process.cwd(),
        `--event=${event}`,
        `--tool=${tool}`,
      ])
      .catch(() => {});
  pi.on("session_start", (_e: unknown, ctx: any) => send("idle", ctx, "session_start"));
  pi.on("agent_start", (_e: unknown, ctx: any) => send("working", ctx, "agent_start"));
  pi.on("tool_execution_start", (e: any, ctx: any) =>
    send("working", ctx, "tool_execution_start", String(e?.toolName ?? e?.tool ?? "")),
  );
  pi.on("agent_end", (_e: unknown, ctx: any) => send("idle", ctx, "agent_end"));
  pi.on("agent_settled", (_e: unknown, ctx: any) => send("idle", ctx, "agent_settled"));
  pi.on("session_shutdown", (_e: unknown, ctx: any) => send("end", ctx, "session_shutdown"));
}
