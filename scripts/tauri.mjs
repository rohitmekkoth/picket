import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { delimiter, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const rustPaths = [
  resolve(homedir(), ".cargo/bin"),
  "/opt/homebrew/opt/rustup/bin",
  "/usr/local/opt/rustup/bin",
].filter(existsSync);
const child = spawn(resolve(root, "node_modules/.bin/tauri"), process.argv.slice(2), {
  cwd: resolve(root, "crates/app"),
  stdio: "inherit",
  env: { ...process.env, PATH: [...rustPaths, process.env.PATH ?? ""].join(delimiter) },
});
child.on("error", (error) => {
  console.error(error.message);
  process.exitCode = 1;
});
child.on("exit", (code, signal) => {
  process.exitCode = code ?? (signal ? 1 : 0);
});
