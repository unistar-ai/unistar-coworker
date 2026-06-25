#!/usr/bin/env node
/**
 * Start unistar-coworker serve for E2E tests.
 * Uses a temp workdir with fixtures/coworker.yaml so the repo root stays clean.
 */
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const E2E_ROOT = path.resolve(__dirname, "..");
const CRATE_ROOT = path.resolve(E2E_ROOT, "..");
const PORT = process.env.E2E_PORT || "18787";
const BIND = `127.0.0.1:${PORT}`;

const workdir = fs.mkdtempSync(path.join(os.tmpdir(), "unistar-e2e-"));
fs.mkdirSync(path.join(workdir, "data"), { recursive: true });
fs.copyFileSync(
  path.join(E2E_ROOT, "fixtures", "coworker.yaml"),
  path.join(workdir, "coworker.yaml"),
);

const binary =
  process.env.UNISTAR_BIN ||
  path.join(CRATE_ROOT, "target", "debug", "unistar-coworker");

if (!fs.existsSync(binary)) {
  console.error(
    `Binary not found at ${binary}. Run: cargo build (from ${CRATE_ROOT})`,
  );
  process.exit(1);
}

fs.writeFileSync(path.join(E2E_ROOT, ".e2e-workdir"), workdir);

const child = spawn(binary, ["serve", "--bind", BIND], {
  cwd: workdir,
  stdio: "inherit",
  env: { ...process.env },
});

child.on("exit", (code, signal) => {
  if (signal) process.kill(process.pid, signal);
  process.exit(code ?? 1);
});

for (const sig of ["SIGINT", "SIGTERM"]) {
  process.on(sig, () => child.kill(sig));
}
