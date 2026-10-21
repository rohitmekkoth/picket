import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { delimiter, resolve } from "node:path";

// Include rustup locations that Homebrew does not add to non-interactive shells.
const rustPaths = [
  resolve(homedir(), ".cargo/bin"),
  "/opt/homebrew/opt/rustup/bin",
  "/usr/local/opt/rustup/bin",
].filter(existsSync);
const env = { ...process.env, PATH: [...rustPaths, process.env.PATH ?? ""].join(delimiter) };
// Fail once after reporting every missing tool in a single run.
const checks = [
  ["node", "--version"],
  ["pnpm", "--version"],
  ["rustc", "--version"],
  ["cargo", "--version"],
  ["cargo", "fmt", "--version"],
  ["cargo", "clippy", "--version"],
  ["cargo-deny", "--version"],
  ["gitleaks", "version"],
  ["actionlint", "-version"],
  ["cmake", "--version"],
  ["git", "--version"],
  ["gh", "--version"],
];
if (process.platform === "darwin") checks.push(["xcrun", "--show-sdk-path"]);
for (const [command, ...args] of checks) {
  const result = spawnSync(command, args, { env, encoding: "utf8" });
  const ok = result.status === 0;
  console.log(
    `${ok ? "OK" : "MISSING"} ${command}: ${(result.stdout || result.stderr || result.error?.message || "unavailable").trim().split("\n")[0]}`,
  );
  if (!ok) process.exitCode = 1;
}
