import { useEffect, useState } from "react";
import { Group, useApp } from "@/App";
import { api, dur, type Day } from "@/lib/api";
import { Button } from "@/components/ui/button";

/** Fixed series slot per agent, so a color always means the same agent (never its rank). */
const AGENTS: { id: string; name: string }[] = [
  { id: "claude", name: "Claude Code" },
  { id: "codex", name: "Codex" },
  { id: "gemini", name: "Gemini CLI" },
  { id: "cursor", name: "Cursor" },
  { id: "copilot", name: "Copilot CLI" },
  { id: "opencode", name: "OpenCode" },
  { id: "pi", name: "Pi" },
  { id: "hermes", name: "Hermes" },
];
const slot = (id: string) => AGENTS.findIndex((a) => a.id === id);
const color = (id: string) => (slot(id) >= 0 ? `var(--series-${slot(id) + 1})` : "var(--series-other)");
const agentName = (id: string) => AGENTS[slot(id)]?.name ?? id;
/** Stack order: known agents in slot order, then process-only agents alphabetically. */
const order = (ids: string[]) => [...ids].sort((a, b) => (slot(a) < 0 ? 99 : slot(a)) - (slot(b) < 0 ? 99 : slot(b)) || a.localeCompare(b));

const total = (d: Day) => Object.values(d.agentSecs).reduce((a, b) => a + b, 0);
const weekday = (date: string, i: number, n: number) =>
  i === n - 1 ? "Today" : new Date(`${date}T12:00:00`).toLocaleDateString([], { weekday: "short" });
const clock = (unix: number) => new Date(unix * 1000).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });

export function ActivitySection() {
  const { status } = useApp();
  const [days, setDays] = useState<Day[]>([]);
  const [hover, setHover] = useState<number | null>(null);
  const [table, setTable] = useState(false);

  useEffect(() => {
    const load = () => api.statsDays(7).then(setDays);
    load();
    const t = setInterval(load, 10_000);
    return () => clearInterval(t);
  }, []);

  const today = days[days.length - 1];
  if (!today) return null;
  const agents = order([...new Set(days.flatMap((d) => Object.keys(d.agentSecs)))]);
  const max = Math.max(...days.map(total), 1);
  const tiles: [string, string][] = [
    ["Agent time", dur(total(today))],
    ["Kept awake", dur(today.heldSecs)],
    ["Turns", String(today.turns)],
    ["Tool calls", String(today.tools)],
    ["Errors", String(today.errors)],
    ["Battery used", status?.battery == null ? "—" : `${today.batteryUsed}%`],
  ];

  return (
    <>
      <Group title="Today">
        <div className="grid grid-cols-3 gap-px overflow-hidden rounded-xl bg-border">
          {tiles.map(([label, value]) => (
            <div key={label} className="bg-card px-4 py-3">
              <div className="text-xs text-muted-foreground">{label}</div>
              <div className="mt-0.5 text-xl font-semibold tabular-nums">{value}</div>
            </div>
          ))}
        </div>
      </Group>

      <Group
        title="Last 7 days"
        footer="Agent time adds up sessions that ran at the same time. Only counted while the app is running."
      >
        <div className="px-4 pt-3 pb-4">
          <div className="mb-3 flex items-start justify-between gap-4">
            <Legend agents={agents} />
            <Button variant="ghost" size="sm" onClick={() => setTable((t) => !t)} aria-pressed={table}>
              {table ? "Chart" : "Table"}
            </Button>
          </div>
          {agents.length === 0 ? (
            <p className="py-8 text-center text-sm text-muted-foreground">No agent activity recorded yet.</p>
          ) : table ? (
            <DaysTable days={days} agents={agents} />
          ) : (
            <div className="relative">
              <div className="flex h-40 items-end gap-3 border-b border-border" onMouseLeave={() => setHover(null)}>
                {days.map((d, i) => {
                  const segments = order(Object.keys(d.agentSecs));
                  return (
                    // The whole column is the hover target: bigger than its segments.
                    <div key={d.date} className="flex h-full flex-1 flex-col justify-end" onMouseEnter={() => setHover(i)}>
                      <div className="flex flex-col-reverse gap-[2px] overflow-hidden rounded-t-[4px]" style={{ height: `${(total(d) / max) * 100}%` }}>
                        {segments.map((a) => (
                          <div key={a} style={{ flexGrow: d.agentSecs[a], background: color(a) }} className="min-h-[2px]" />
                        ))}
                      </div>
                    </div>
                  );
                })}
              </div>
              <div className="mt-1.5 flex gap-3">
                {days.map((d, i) => (
                  <div key={d.date} className={`flex-1 text-center text-xs ${i === hover ? "text-foreground" : "text-muted-foreground"}`}>
                    {weekday(d.date, i, days.length)}
                  </div>
                ))}
              </div>
              <span className="absolute top-0 left-0 text-[10px] text-muted-foreground tabular-nums">{dur(max)}</span>
              {hover !== null && total(days[hover]) > 0 && (
                <Tooltip day={days[hover]} label={weekday(days[hover].date, hover, days.length)} at={hover} of={days.length} />
              )}
            </div>
          )}
        </div>
      </Group>

      <Group title="Today's sessions">
        {today.sessions.length === 0 ? (
          <p className="px-4 py-3 text-sm text-muted-foreground">No sessions yet today.</p>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-xs">
              <thead>
                <tr className="text-left text-muted-foreground">
                  {["Session", "Working", "Turns", "Tools", "Errors"].map((h, i) => (
                    <th key={h} className={`px-2 py-2 font-normal whitespace-nowrap first:pl-4 last:pr-4 ${i >= 1 ? "text-right" : ""}`}>
                      {h}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody className="divide-y">
                {[...today.sessions]
                  .sort((a, b) => b.firstSeen - a.firstSeen)
                  .map((s) => (
                    <tr key={s.key} className="tabular-nums">
                      {/* w-full + max-w-0: this column takes the leftover width and truncates. */}
                      <td className="w-full max-w-0 py-2 pr-2 pl-4">
                        <div className="flex items-center gap-2 truncate text-sm whitespace-nowrap">
                          <span className="size-2 shrink-0 rounded-full" style={{ background: color(s.agent) }} />
                          {s.name || agentName(s.agent)}
                        </div>
                        <div className="truncate pl-4 text-muted-foreground">
                          {s.project ? `${s.project} · ` : ""}since {clock(s.firstSeen)}
                        </div>
                      </td>
                      <td className="px-2 py-2 text-right whitespace-nowrap">{dur(s.workingSecs)}</td>
                      <td className="px-2 py-2 text-right">{s.turns}</td>
                      <td className="px-2 py-2 text-right">{s.tools}</td>
                      <td className="py-2 pr-4 pl-2 text-right">{s.errors}</td>
                    </tr>
                  ))}
              </tbody>
            </table>
          </div>
        )}
      </Group>
    </>
  );
}

function Legend({ agents }: { agents: string[] }) {
  return (
    <ul className="flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
      {agents.map((a) => (
        <li key={a} className="flex items-center gap-1.5">
          <span className="size-2.5 rounded-[3px]" style={{ background: color(a) }} />
          {agentName(a)}
        </li>
      ))}
    </ul>
  );
}

/** Floats beside the hovered column, on whichever side has room, so it never covers it. */
function Tooltip({ day, label, at, of }: { day: Day; label: string; at: number; of: number }) {
  const place = at < of / 2 ? { left: `calc(${((at + 1) / of) * 100}% + 4px)` } : { right: `calc(${((of - at) / of) * 100}% + 4px)` };
  return (
    <div className="pointer-events-none absolute top-2 z-10 min-w-44 rounded-lg border bg-popover px-3 py-2 text-xs shadow-md" style={place}>
      <div className="mb-1 font-medium">
        {label} · {dur(total(day))}
      </div>
      {order(Object.keys(day.agentSecs)).map((a) => (
        <div key={a} className="flex items-center justify-between gap-4 tabular-nums">
          <span className="flex items-center gap-1.5 text-muted-foreground">
            <span className="size-2 rounded-[2px]" style={{ background: color(a) }} />
            {agentName(a)}
          </span>
          {dur(day.agentSecs[a])}
        </div>
      ))}
      <div className="mt-1 text-muted-foreground">Kept awake {dur(day.heldSecs)}</div>
    </div>
  );
}

function DaysTable({ days, agents }: { days: Day[]; agents: string[] }) {
  return (
    <div className="overflow-x-auto">
      <table className="w-full text-xs tabular-nums">
        <thead>
          <tr className="text-left text-muted-foreground">
            <th className="py-1 pr-3 font-normal">Day</th>
            {agents.map((a) => (
              <th key={a} className="py-1 pr-3 text-right font-normal">
                {agentName(a)}
              </th>
            ))}
            <th className="py-1 pr-3 text-right font-normal">Kept awake</th>
          </tr>
        </thead>
        <tbody>
          {days.map((d, i) => (
            <tr key={d.date} className="border-t">
              <td className="py-1 pr-3">{weekday(d.date, i, days.length)}</td>
              {agents.map((a) => (
                <td key={a} className="py-1 pr-3 text-right">
                  {d.agentSecs[a] ? dur(d.agentSecs[a]) : "—"}
                </td>
              ))}
              <td className="py-1 pr-3 text-right">{d.heldSecs ? dur(d.heldSecs) : "—"}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
