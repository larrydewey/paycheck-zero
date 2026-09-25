import { spawnSync } from "node:child_process";
import { WEBKIT_CONTAINER } from "./global-setup";

export default function globalTeardown() {
  if (process.env.PZ_WEBKIT_WS) spawnSync("docker", ["rm", "-f", WEBKIT_CONTAINER], { stdio: "ignore" });
}
