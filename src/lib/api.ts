import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type DisplayOff = "never" | "onLidClose" | "whileAgentsRun" | "afterFinish";

export interface Settings {
  enabled: boolean;
  pausedUntil: number;
  batteryCutoff: number;
  onlyPluggedIn: boolean;
  respectLowPower: boolean;
  thermalLimit: number;
  displayOff: DisplayOff;
  displayOffAfterSecs: number;
  lockOnLidClose: boolean;
  keepDisplayOn: boolean;
  notifyEngage: boolean;
  notifyFinish: boolean;
  notifyBattery: boolean;
  sound: string;
  shortcut: string;
  launchAtLogin: boolean;
  processAgents: string[];
  firstRun: number;
}

export type Reason =
  | "disabled"
  | "paused"
  | "noAgents"
  | "thermal"
  | "lowPower"
  | "notPluggedIn"
  | "battery"
  | "holding";

export interface Session {
  agent: string;
  name: string;
  id: string;
  state: "working" | "waiting" | "idle";
  project: string;
}

export interface Platform {
  os: "macos" | "windows" | "linux";
  /** macOS needs a one-time admin grant for lid-closed awake. */
  needsGrant: boolean;
  /** Whether this OS reports thermal pressure (not on Windows). */
  thermal: boolean;
}

export interface Status {
  reason: Reason;
  held: boolean;
  heldSecs: number;
  lidProof: boolean;
  battery: number | null;
  onAc: boolean;
  thermal: number | null;
  lowPower: boolean;
  lidClosed: boolean;
  working: number;
  sessions: Session[];
  processAgents: string[];
  pausedUntil: number;
  platform: Platform;
}

export interface AgentStatus {
  id: string;
  name: string;
  state: "notDetected" | "available" | "installed";
  note: string;
}

export const api = {
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<void>("save_settings", { settings }),
  getStatus: () => invoke<Status | null>("get_status"),
  agents: () => invoke<AgentStatus[]>("agents_status"),
  installAgent: (id: string) => invoke<void>("install_agent", { id }),
  uninstallAgent: (id: string) => invoke<void>("uninstall_agent", { id }),
  removeAllIntegrations: () => invoke<void>("remove_all_integrations"),
  installGrant: () => invoke<void>("install_grant"),
  uninstallGrant: () => invoke<void>("uninstall_grant"),
  sounds: () => invoke<string[]>("sounds"),
  previewSound: (name: string) => invoke<void>("preview_sound", { name }),
  onStatus: (cb: (s: Status) => void) => listen<Status>("status", (e) => cb(e.payload)),
  /** Fired whenever settings change, including from the tray menu or ⌥⌘L. */
  onSettings: (cb: (s: Settings) => void) => listen<Settings>("settings", (e) => cb(e.payload)),
};

/** "Alt+Super+KeyL" → "⌥⌘L" on macOS, "Ctrl+Alt+Shift+L" elsewhere. */
export function prettyShortcut(accel: string, os: Platform["os"] = "macos") {
  const mac: Record<string, string> = { alt: "⌥", option: "⌥", super: "⌘", cmd: "⌘", command: "⌘", shift: "⇧", control: "⌃", ctrl: "⌃" };
  const pc: Record<string, string> = { alt: "Alt", super: "Win", cmd: "Win", command: "Win", shift: "Shift", control: "Ctrl", ctrl: "Ctrl" };
  const map = os === "macos" ? mac : pc;
  return accel
    .split("+")
    .map((t) => map[t.toLowerCase()] ?? t.replace(/^Key|^Digit/, "").toUpperCase())
    .join(os === "macos" ? "" : "+");
}

/** "Mac" / "PC" / "computer", for copy. */
export const deviceName = (os?: Platform["os"]) => (os === "macos" ? "Mac" : os === "windows" ? "PC" : "computer");
