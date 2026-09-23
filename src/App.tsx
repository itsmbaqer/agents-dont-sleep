import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { BatteryMedium, Bell, Bot, KeyRound, Monitor, Power, type LucideIcon } from "lucide-react";
import { cn } from "@/lib/utils";
import { api, type Settings, type Status } from "@/lib/api";
import { AgentsSection, DisplaySection, GeneralSection, LicenseSection, NotificationsSection, PowerSection } from "@/sections";

interface Ctx {
  settings: Settings;
  status: Status | null;
  update: (patch: Partial<Settings>) => void;
  run: (p: Promise<unknown>, ok?: string) => Promise<boolean>;
}

const AppCtx = createContext<Ctx>(null!);
export const useApp = () => useContext(AppCtx);

const SECTIONS: { id: string; label: string; icon: LucideIcon; view: () => ReactNode }[] = [
  { id: "general", label: "General", icon: Power, view: GeneralSection },
  { id: "agents", label: "Agents", icon: Bot, view: AgentsSection },
  { id: "power", label: "Power", icon: BatteryMedium, view: PowerSection },
  { id: "display", label: "Display", icon: Monitor, view: DisplaySection },
  { id: "notifications", label: "Notifications", icon: Bell, view: NotificationsSection },
  { id: "license", label: "License", icon: KeyRound, view: LicenseSection },
];

export default function App() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [status, setStatus] = useState<Status | null>(null);
  const [section, setSection] = useState("general");
  const [flash, setFlash] = useState<{ text: string; error: boolean } | null>(null);

  const settingsRef = useRef<Settings | null>(null);
  const apply = useCallback((s: Settings) => {
    settingsRef.current = s;
    setSettings(s);
  }, []);

  useEffect(() => {
    api.getSettings().then(apply);
    api.getStatus().then(setStatus);
    const offs = [api.onStatus(setStatus), api.onSettings(apply)];
    return () => offs.forEach((off) => off.then((f) => f()));
  }, [apply]);

  useEffect(() => {
    if (!flash) return;
    const t = setTimeout(() => setFlash(null), flash.error ? 8000 : 2500);
    return () => clearTimeout(t);
  }, [flash]);

  const run = useCallback(async (p: Promise<unknown>, ok?: string) => {
    try {
      await p;
      if (ok) setFlash({ text: ok, error: false });
      return true;
    } catch (e) {
      setFlash({ text: String(e), error: true });
      return false;
    }
  }, []);

  const update = useCallback(
    (patch: Partial<Settings>) => {
      const next = { ...settingsRef.current!, ...patch };
      apply(next);
      // Rust validates; on rejection reload the truth.
      run(api.saveSettings(next)).then((ok) => {
        if (!ok) api.getSettings().then(apply);
      });
    },
    [run, apply],
  );

  if (!settings) return null;
  const View = SECTIONS.find((s) => s.id === section)!.view;

  return (
    <AppCtx.Provider value={{ settings, status, update, run }}>
      <div className="flex h-screen bg-background text-foreground select-none">
        <nav className="w-52 shrink-0 border-r bg-muted/40 p-3 pt-4">
          <StatusPill status={status} />
          <ul className="mt-4 space-y-0.5">
            {SECTIONS.map(({ id, label, icon: Icon }) => (
              <li key={id}>
                <button
                  onClick={() => setSection(id)}
                  className={cn(
                    "flex w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-sm transition-colors",
                    section === id ? "bg-primary text-primary-foreground" : "hover:bg-accent",
                  )}
                >
                  <Icon className="size-4" />
                  {label}
                </button>
              </li>
            ))}
          </ul>
        </nav>
        <main className="relative flex-1 overflow-y-auto p-6">
          <h1 className="mb-5 text-lg font-semibold">{SECTIONS.find((s) => s.id === section)!.label}</h1>
          <div className="space-y-6">
            <View />
          </div>
          {flash && (
            <div
              role="status"
              className={cn(
                "fixed right-4 bottom-4 max-w-md rounded-lg border px-4 py-2.5 text-sm whitespace-pre-wrap shadow-lg select-text",
                flash.error ? "border-destructive/40 bg-destructive/10 text-destructive" : "bg-popover",
              )}
            >
              {flash.text}
            </div>
          )}
        </main>
      </div>
    </AppCtx.Provider>
  );
}

function StatusPill({ status }: { status: Status | null }) {
  const held = status?.held ?? false;
  const mins = Math.floor((status?.heldSecs ?? 0) / 60);
  return (
    <div className="rounded-lg border bg-card p-3">
      <div className="flex items-center gap-2 text-sm font-medium">
        <span className={cn("size-2 rounded-full", held ? "bg-emerald-500" : "bg-muted-foreground/40")} />
        {held ? "Awake" : "Idle"}
      </div>
      <p className="mt-1 text-xs text-muted-foreground">
        {held
          ? `${status!.working} working · ${Math.floor(mins / 60)}h ${mins % 60}m`
          : status?.reason === "noAgents" || !status
            ? "Your Mac can sleep normally"
            : REASON_TEXT[status.reason]}
      </p>
      {status?.battery != null && (
        <p className="mt-1 text-xs text-muted-foreground">
          Battery {status.battery}%{status.onAc ? " · on power" : ""}
        </p>
      )}
    </div>
  );
}

export const REASON_TEXT: Record<Status["reason"], string> = {
  holding: "Keeping your Mac awake",
  noAgents: "No agents working",
  disabled: "Turned off",
  paused: "Paused",
  battery: "Held off: battery limit",
  thermal: "Held off: running hot",
  lowPower: "Held off: Low Power Mode",
  notPluggedIn: "Held off: not plugged in",
  unlicensed: "Trial ended",
};

export function Group({ title, footer, children }: { title?: string; footer?: ReactNode; children: ReactNode }) {
  return (
    <section>
      {title && <h2 className="mb-2 px-1 text-xs font-medium tracking-wide text-muted-foreground uppercase">{title}</h2>}
      <div className="divide-y rounded-xl border bg-card">{children}</div>
      {footer && <p className="mt-2 px-1 text-xs text-muted-foreground">{footer}</p>}
    </section>
  );
}

export function Row({ title, hint, children }: { title: ReactNode; hint?: ReactNode; children?: ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-6 px-4 py-3">
      <div className="min-w-0">
        <div className="text-sm font-medium">{title}</div>
        {hint && <div className="mt-0.5 text-xs text-muted-foreground">{hint}</div>}
      </div>
      {children && <div className="flex shrink-0 items-center gap-2">{children}</div>}
    </div>
  );
}
