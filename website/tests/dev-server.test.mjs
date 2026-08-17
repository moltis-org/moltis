import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import test from "node:test";

test("serves the website on the advertised loopback URL", { timeout: 10_000 }, async () => {
  const child = spawn(process.execPath, ["scripts/dev-server.mjs"], {
    cwd: new URL("..", import.meta.url),
    env: { ...process.env, HOST: "127.0.0.1", PORT: "0" },
    stdio: ["ignore", "pipe", "pipe"],
  });

  try {
    const url = await new Promise((resolve, reject) => {
      child.once("error", reject);
      child.stderr.once("data", (data) => reject(new Error(data.toString())));
      child.stdout.once("data", (data) => {
        const match = data.toString().match(/Website dev server: (http:\/\/127\.0\.0\.1:\d+)/);
        match ? resolve(match[1]) : reject(new Error(`Unexpected server output: ${data}`));
      });
    });

    const response = await fetch(url);
    assert.equal(response.status, 200);
    assert.match(await response.text(), /<title>Moltis/);
  } finally {
    child.kill();
  }
});
