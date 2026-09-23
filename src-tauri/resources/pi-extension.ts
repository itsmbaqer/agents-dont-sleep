// Agents Don't Sleep: reports pi's agent lifecycle to the menu-bar app. Safe to delete;
// reinstall from Settings → Agents.
const HOOK = __HOOK__;

export default function (pi: any) {
  const send = (state: string, ctx: any) =>
    pi
      .exec(HOOK, ["pi", state, String(ctx?.sessionManager?.getSessionId?.() ?? `pid-${process.pid}`), process.cwd()])
      .catch(() => {});
  pi.on("session_start", (_e: unknown, ctx: any) => send("idle", ctx));
  pi.on("agent_start", (_e: unknown, ctx: any) => send("working", ctx));
  pi.on("agent_end", (_e: unknown, ctx: any) => send("idle", ctx));
  pi.on("agent_settled", (_e: unknown, ctx: any) => send("idle", ctx));
  pi.on("session_shutdown", (_e: unknown, ctx: any) => send("end", ctx));
}
