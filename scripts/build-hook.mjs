// Builds the `adshook` sidecar for the current Tauri target into src-tauri/binaries/, where
// `bundle.externalBin` expects it (adshook-<target-triple>[.exe]). Runs before dev and build.
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";

const tauriDir = fileURLToPath(new URL("../src-tauri/", import.meta.url));
const host = execFileSync("rustc", ["-vV"]).toString().match(/host: (\S+)/)[1];
const triple = process.env.TAURI_ENV_TARGET_TRIPLE || host;
const exe = triple.includes("windows") ? ".exe" : "";

execFileSync("cargo", ["build", "--release", "-p", "ads-hook", "--target", triple], { cwd: tauriDir, stdio: "inherit" });
mkdirSync(`${tauriDir}binaries`, { recursive: true });
copyFileSync(`${tauriDir}target/${triple}/release/adshook${exe}`, `${tauriDir}binaries/adshook-${triple}${exe}`);
console.log(`adshook ready for ${triple}`);
