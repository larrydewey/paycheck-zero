import { execFileSync, spawnSync } from "node:child_process";
import path from "node:path";
import { webkit } from "@playwright/test";

export const WEBKIT_CONTAINER = "pz-playwright-webkit";
const IMAGE = "mcr.microsoft.com/playwright:v1.63.0-noble";
const PORT = 53333;

/**
 * 1. Builds the server binary once before any worker starts it.
 * 2. Playwright's WebKit build needs Ubuntu 24.04 system libraries. When it
 *    cannot launch on this host, start Playwright's official Docker image as a
 *    remote browser server (host networking) so mobile-Safari emulation still
 *    runs with the same single `npm test` command.
 */
export default async function globalSetup() {
  const root = path.resolve(__dirname, "..");
  execFileSync("cargo", ["build", "-p", "paycheckzero-web", "--bin", "paycheckzero"], { cwd: root, stdio: "inherit" });

  if (process.env.PZ_WEBKIT === "local") return;
  try {
    const b = await webkit.launch();
    await b.close();
    return;
  } catch {
    /* fall through to Docker */
  }
  spawnSync("docker", ["rm", "-f", WEBKIT_CONTAINER], { stdio: "ignore" });
  const run = spawnSync("docker", [
    "run", "-d", "--rm", "--name", WEBKIT_CONTAINER, "--network", "host", "--init", "--ipc", "host", IMAGE,
    "/bin/sh", "-c", `cd /tmp && npx -y playwright@1.63.0 run-server --port ${PORT} --host 127.0.0.1`,
  ], { encoding: "utf8" });
  if (run.status !== 0) {
    console.warn("WebKit cannot run locally and Docker is unavailable; mobile-safari tests will fail.\n" + run.stderr);
    return;
  }
  const ws = `ws://127.0.0.1:${PORT}/`;
  const deadline = Date.now() + 120_000;
  for (;;) {
    try {
      const b = await webkit.connect(ws);
      await b.close();
      break;
    } catch (e) {
      if (Date.now() > deadline) throw new Error("WebKit browser server did not start: " + e);
      await new Promise((r) => setTimeout(r, 1000));
    }
  }
  process.env.PZ_WEBKIT_WS = ws;
}
