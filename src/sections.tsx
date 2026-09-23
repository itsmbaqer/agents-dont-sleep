import { useEffect, useState, type KeyboardEvent } from "react";
import { Play } from "lucide-react";
import { Group, Row, useApp } from "@/App";
import { api, prettyShortcut, type AgentStatus, type DisplayOff } from "@/lib/api";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Slider } from "@/components/ui/slider";
import { Switch } from "@/components/ui/switch";

export function GeneralSection() {
  const { settings, status, update, run } = useApp();
  const lidProof = status?.lidProof ?? false;
  return (
    <>
      <Group>
        <Row title="Keep my Mac awake while agents work" hint={`Toggle from anywhere with ${prettyShortcut(settings.shortcut)}.`}>
          <Switch checked={settings.enabled} onCheckedChange={(enabled) => update({ enabled })} aria-label="Enabled" />
        </Row>
        {settings.pausedUntil * 1000 > Date.now() && (
          <Row title="Paused" hint={`Until ${new Date(settings.pausedUntil * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}`}>
            <Button variant="outline" size="sm" onClick={() => update({ pausedUntil: 0 })}>
              Resume
            </Button>
          </Row>
        )}
        <Row title="Toggle shortcut" hint="Click, then press the new key combination.">
          <ShortcutInput value={settings.shortcut} onChange={(shortcut) => update({ shortcut })} />
        </Row>
        <Row title="Launch at login" hint="Also makes sure a crash or restart never leaves sleep disabled.">
          <Switch checked={settings.launchAtLogin} onCheckedChange={(launchAtLogin) => update({ launchAtLogin })} aria-label="Launch at login" />
        </Row>
      </Group>

      <Group
        title="Lid-closed awake"
        footer="macOS always sleeps when the lid closes. Staying awake takes the system SleepDisabled switch (pmset disablesleep), which needs admin rights once. The rule allows exactly that command and nothing else, and is only switched on while an agent is working."
      >
        <Row
          title={
            <span className="flex items-center gap-2">
              Permission {lidProof ? <Badge>Granted</Badge> : <Badge variant="outline">Not granted</Badge>}
            </span>
          }
          hint={lidProof ? "Your Mac keeps working with the lid shut." : "Without it, agents are only covered while the lid is open."}
        >
          {lidProof ? (
            <Button variant="outline" size="sm" onClick={() => run(api.uninstallSudoers(), "Permission removed.")}>
              Revoke
            </Button>
          ) : (
            <Button size="sm" onClick={() => run(api.installSudoers(), "Lid-closed awake is ready.")}>
              Grant…
            </Button>
          )}
        </Row>
      </Group>
    </>
  );
}

function ShortcutInput({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  const [recording, setRecording] = useState(false);
  const onKeyDown = (e: KeyboardEvent) => {
    e.preventDefault();
    if (e.key === "Escape") return setRecording(false);
    if (["Meta", "Alt", "Shift", "Control"].includes(e.key)) return;
    const mods = [e.ctrlKey && "Control", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Super"].filter(Boolean);
    if (!mods.length) return; // a bare key would hijack typing everywhere
    onChange([...mods, e.code].join("+"));
    setRecording(false);
  };
  return (
    <Button variant="outline" size="sm" className="min-w-24 font-mono" onClick={() => setRecording(true)} onKeyDown={recording ? onKeyDown : undefined} onBlur={() => setRecording(false)}>
      {recording ? "Press keys…" : prettyShortcut(value)}
    </Button>
  );
}

const STATE_BADGE: Record<AgentStatus["state"], { label: string; variant: "default" | "secondary" | "outline" }> = {
  installed: { label: "Connected", variant: "default" },
  available: { label: "Found", variant: "secondary" },
  notDetected: { label: "Not found", variant: "outline" },
};

export function AgentsSection() {
  const { settings, status, update, run } = useApp();
  const [agents, setAgents] = useState<AgentStatus[]>([]);
  const [procs, setProcs] = useState(settings.processAgents.join(", "));
  const refresh = () => api.agents().then(setAgents);
  useEffect(() => void refresh(), []);

  const act = async (a: AgentStatus) => {
    const install = a.state !== "installed";
    await run(install ? api.installAgent(a.id) : api.uninstallAgent(a.id), install ? `${a.name} connected.${a.note ? ` ${a.note}` : ""}` : `${a.name} disconnected.`);
    refresh();
  };

  return (
    <>
      <Group title="Lifecycle hooks" footer="Hooks tell the app exactly when an agent starts and finishes a turn. They only record state; they never read your code, prompts, or output.">
        {agents.map((a) => {
          const live = status?.sessions.filter((s) => s.agent === a.id) ?? [];
          const working = live.filter((s) => s.state !== "idle").length;
          return (
            <Row
              key={a.id}
              title={
                <span className="flex items-center gap-2">
                  {a.name}
                  <Badge variant={STATE_BADGE[a.state].variant}>{STATE_BADGE[a.state].label}</Badge>
                  {live.length > 0 && (
                    <span className="text-xs font-normal text-muted-foreground">
                      {live.length} session{live.length > 1 ? "s" : ""}
                      {working ? ` · ${working} working` : ""}
                    </span>
                  )}
                </span>
              }
              hint={a.note || undefined}
            >
              <Button size="sm" variant={a.state === "installed" ? "outline" : "default"} disabled={a.state === "notDetected"} onClick={() => act(a)}>
                {a.state === "installed" ? "Disconnect" : "Connect"}
              </Button>
            </Row>
          );
        })}
      </Group>

      <Group title="Process detection" footer="For agents without hooks: counted as working for as long as a process with one of these names runs.">
        <div className="px-4 py-3">
          <Label htmlFor="procs" className="mb-2 block text-sm">
            Process names
          </Label>
          <Input
            id="procs"
            value={procs}
            onChange={(e) => setProcs(e.target.value)}
            onBlur={() => update({ processAgents: procs.split(",").map((p) => p.trim()).filter(Boolean) })}
            placeholder="aider, goose"
            spellCheck={false}
          />
          {status?.processAgents.length ? <p className="mt-2 text-xs text-muted-foreground">Running now: {status.processAgents.join(", ")}</p> : null}
        </div>
      </Group>
    </>
  );
}

export function PowerSection() {
  const { settings, status, update } = useApp();
  const [cutoff, setCutoff] = useState(settings.batteryCutoff);
  useEffect(() => setCutoff(settings.batteryCutoff), [settings.batteryCutoff]);
  return (
    <>
      <Group title="Battery">
        <Row title="Stop below" hint="Releases the wake lock and lets your Mac sleep when the battery gets this low.">
          <div className="flex w-56 items-center gap-3">
            <Slider min={5} max={50} step={5} value={[cutoff]} onValueChange={([v]) => setCutoff(v)} onValueCommit={([batteryCutoff]) => update({ batteryCutoff })} aria-label="Battery cut-off" />
            <span className="w-10 text-right text-sm tabular-nums">{cutoff}%</span>
          </div>
        </Row>
        <Row title="Only when plugged in" hint="Never hold the Mac awake on battery.">
          <Switch checked={settings.onlyPluggedIn} onCheckedChange={(onlyPluggedIn) => update({ onlyPluggedIn })} aria-label="Only when plugged in" />
        </Row>
        <Row title="Respect Low Power Mode" hint={status?.lowPower ? "Low Power Mode is on right now." : "Stand down while Low Power Mode is on."}>
          <Switch checked={settings.respectLowPower} onCheckedChange={(respectLowPower) => update({ respectLowPower })} aria-label="Respect Low Power Mode" />
        </Row>
      </Group>

      <Group title="Heat" footer="Always on. A closed MacBook in a bag can't shed heat, so the app steps back when macOS reports thermal pressure and re-engages once it cools.">
        <Row title="Stop at thermal level">
          <Select value={String(settings.thermalLimit)} onValueChange={(v) => update({ thermalLimit: Number(v) })}>
            <SelectTrigger className="w-56" aria-label="Thermal limit">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="2">Serious (recommended)</SelectItem>
              <SelectItem value="3">Critical</SelectItem>
            </SelectContent>
          </Select>
        </Row>
      </Group>
    </>
  );
}

const DISPLAY_OPTIONS: { value: DisplayOff; label: string; hint: string }[] = [
  { value: "onLidClose", label: "When the lid closes", hint: "Blanks the screen the moment you shut the lid." },
  { value: "whileAgentsRun", label: "While agents run", hint: "Turns the display off as soon as an agent starts working." },
  { value: "afterFinish", label: "After agents finish", hint: "Turns it off a while after the last agent finishes, if you're away." },
  { value: "never", label: "Never", hint: "Leave the display to macOS." },
];

export function DisplaySection() {
  const { settings, update } = useApp();
  const [secs, setSecs] = useState(String(settings.displayOffAfterSecs));
  return (
    <>
      <Group title="Turn the display off" footer="Display rules on lid close only apply without an external display. At a desk in clamshell mode, your monitor keeps working as normal.">
        <RadioGroup value={settings.displayOff} onValueChange={(v) => update({ displayOff: v as DisplayOff })} className="gap-0 divide-y">
          {DISPLAY_OPTIONS.map((o) => (
            <Label key={o.value} htmlFor={`d-${o.value}`} className="flex cursor-pointer items-start gap-3 px-4 py-3 font-normal">
              <RadioGroupItem id={`d-${o.value}`} value={o.value} className="mt-0.5" />
              <div className="flex-1">
                <div className="text-sm font-medium">{o.label}</div>
                <div className="mt-0.5 text-xs text-muted-foreground">{o.hint}</div>
                {o.value === "afterFinish" && settings.displayOff === "afterFinish" && (
                  <div className="mt-2 flex items-center gap-2 text-xs">
                    <Input
                      type="number"
                      min={5}
                      max={3600}
                      value={secs}
                      onChange={(e) => setSecs(e.target.value)}
                      onBlur={() => update({ displayOffAfterSecs: Math.min(3600, Math.max(5, Number(secs) || 30)) })}
                      className="h-7 w-20"
                      aria-label="Seconds after finishing"
                    />
                    seconds
                  </div>
                )}
              </div>
            </Label>
          ))}
        </RadioGroup>
      </Group>

      <Group>
        <Row title="Lock the screen when the lid closes" hint="Staying awake skips the usual lock-on-sleep, so the app locks for you.">
          <Switch checked={settings.lockOnLidClose} onCheckedChange={(lockOnLidClose) => update({ lockOnLidClose })} aria-label="Lock on lid close" />
        </Row>
      </Group>
    </>
  );
}

export function NotificationsSection() {
  const { settings, update } = useApp();
  const [sounds, setSounds] = useState<string[]>([]);
  useEffect(() => void api.sounds().then(setSounds), []);
  return (
    <>
      <Group title="Notify me" footer="Heat warnings are always shown.">
        <Row title="When it starts keeping the Mac awake">
          <Switch checked={settings.notifyEngage} onCheckedChange={(notifyEngage) => update({ notifyEngage })} aria-label="Notify on engage" />
        </Row>
        <Row title="When agents finish">
          <Switch checked={settings.notifyFinish} onCheckedChange={(notifyFinish) => update({ notifyFinish })} aria-label="Notify on finish" />
        </Row>
        <Row title="When the battery limit is reached">
          <Switch checked={settings.notifyBattery} onCheckedChange={(notifyBattery) => update({ notifyBattery })} aria-label="Notify on battery limit" />
        </Row>
      </Group>
      <Group>
        <Row title="Sound">
          <Select value={settings.sound || "none"} onValueChange={(v) => update({ sound: v === "none" ? "" : v })}>
            <SelectTrigger className="w-40" aria-label="Notification sound">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="none">None</SelectItem>
              {sounds.map((s) => (
                <SelectItem key={s} value={s}>
                  {s}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Button variant="outline" size="icon-sm" disabled={!settings.sound} onClick={() => api.previewSound(settings.sound)} aria-label="Preview sound">
            <Play />
          </Button>
        </Row>
      </Group>
    </>
  );
}

export function LicenseSection() {
  const { status, run } = useApp();
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const lic = status?.license;
  if (!lic) return null;

  if (!lic.configured) {
    return (
      <Group footer="Set POLAR_ORG_ID in src-tauri/src/license.rs to sell this build with Polar license keys (a 7-day trial, up to 3 Macs per key).">
        <Row title="Licensing is off" hint="This build runs without a license." />
      </Group>
    );
  }

  const activate = async () => {
    setBusy(true);
    if (await run(api.activateLicense(key), "License activated. Thank you!")) setKey("");
    setBusy(false);
  };

  return (
    <>
      <Group>
        <Row
          title={
            <span className="flex items-center gap-2">
              Status
              {lic.state === "licensed" && <Badge>Licensed</Badge>}
              {lic.state === "trial" && <Badge variant="secondary">Trial · {lic.trialDaysLeft} day{lic.trialDaysLeft === 1 ? "" : "s"} left</Badge>}
              {lic.state === "expired" && <Badge variant="destructive">Trial ended</Badge>}
            </span>
          }
          hint={lic.state === "licensed" ? "Lifetime license, up to 3 Macs. Keys are managed in your Polar account." : "One-time purchase, lifetime license for up to 3 Macs."}
        >
          {lic.state === "licensed" ? (
            <Button variant="outline" size="sm" onClick={() => run(api.deactivateLicense(), "This Mac was removed from your license.")}>
              Deactivate this Mac
            </Button>
          ) : (
            <Button size="sm" onClick={() => api.openUrl(lic.buyUrl)}>
              Buy a license
            </Button>
          )}
        </Row>
      </Group>
      {lic.state !== "licensed" && (
        <Group title="Enter license key">
          <div className="flex gap-2 px-4 py-3">
            <Input value={key} onChange={(e) => setKey(e.target.value)} placeholder="XXXX-XXXX-XXXX-XXXX" spellCheck={false} className="font-mono" onKeyDown={(e) => e.key === "Enter" && key && activate()} />
            <Button disabled={!key || busy} onClick={activate}>
              {busy ? "Activating…" : "Activate"}
            </Button>
          </div>
        </Group>
      )}
    </>
  );
}
