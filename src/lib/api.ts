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
  notifyEngage: boolean;
  notifyFinish: boolean;
  notifyBattery: boolean;
  sound: string;
  shortcut: string;
  launchAtLogin: boolean;
  processAgents: string[];
  firstRun: number;
  licenseKey: string;
  activationId: string;
  licenseOk: boolean;
  licenseCheckedAt: number;
}

export type Reason =
  | "unlicensed"
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

export interface LicenseInfo {
  configured: boolean;
  state: "licensed" | "trial" | "expired" | "off";
  trialDaysLeft: number;
  buyUrl: string;
}

export interface Status {
  reason: Reason;
  held: boolean;
  heldSecs: number;
  lidProof: boolean;
  battery: number | null;
  onAc: boolean;
  thermal: number;
  lowPower: boolean;
  lidClosed: boolean;
  working: number;
  sessions: Session[];
  processAgents: string[];
  pausedUntil: number;
  license: LicenseInfo;
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
  installSudoers: () => invoke<void>("install_sudoers"),
  uninstallSudoers: () => invoke<void>("uninstall_sudoers"),
  sounds: () => invoke<string[]>("sounds"),
  previewSound: (name: string) => invoke<void>("preview_sound", { name }),
  activateLicense: (key: string) => invoke<void>("activate_license", { key }),
  deactivateLicense: () => invoke<void>("deactivate_license"),
  openUrl: (url: string) => invoke<void>("open_url", { url }),
  onStatus: (cb: (s: Status) => void) => listen<Status>("status", (e) => cb(e.payload)),
  /** Fired whenever settings change, including from the tray menu or ⌥⌘L. */
  onSettings: (cb: (s: Settings) => void) => listen<Settings>("settings", (e) => cb(e.payload)),
};

/** "Alt+Super+KeyL" → "⌥⌘L" */
export function prettyShortcut(accel: string) {
  const map: Record<string, string> = { alt: "⌥", option: "⌥", super: "⌘", cmd: "⌘", command: "⌘", shift: "⇧", control: "⌃", ctrl: "⌃" };
  return accel
    .split("+")
    .map((t) => map[t.toLowerCase()] ?? t.replace(/^Key|^Digit/, "").toUpperCase())
    .join("");
}
