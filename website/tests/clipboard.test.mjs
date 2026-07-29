import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { JSDOM } from "jsdom";

const html = readFileSync(new URL("../index.en.html", import.meta.url), "utf8");
const clipboardScript = readFileSync(new URL("../clipboard.js", import.meta.url), "utf8");

test("loads clipboard behavior after the copy controls", () => {
  assert.ok(html.indexOf('<script src="clipboard.js"></script>') > html.indexOf('data-copy-text="curl'));
});

test("offers copy controls for every primary install method", () => {
  const { dom, window } = setup();
  const commands = [...window.document.querySelectorAll("button[data-copy-text]")]
    .map((button) => button.dataset.copyText);

  assert.deepEqual(commands, [
    "curl -fsSL moltis.org/install.sh | sh",
    "curl -fsSL https://www.moltis.org/install.sh | sh",
    "brew install moltis-org/tap/moltis",
    "docker run -d \\\n    --name moltis -p 13131:13131 -p 1455:1455 \\\n    -v moltis-config:/home/moltis/.config/moltis \\\n    -v moltis-data:/home/moltis/.moltis \\\n    -v /var/run/docker.sock:/var/run/docker.sock \\\n    ghcr.io/moltis-org/moltis:latest",
    "cargo install moltis --git https://github.com/moltis-org/moltis",
  ]);
  dom.window.close();
});

function setup() {
  const dom = new JSDOM(html, { runScripts: "outside-only" });
  return { dom, window: dom.window };
}

test("copies the exact install command and shows success feedback", async () => {
  const { dom, window } = setup();
  const writes = [];
  Object.defineProperty(window.navigator, "clipboard", {
    value: { writeText: async (text) => writes.push(text) },
  });
  window.eval(clipboardScript);

  const button = window.document.querySelector('button[data-copy-text^="curl -fsSL moltis.org"]');
  button.click();
  await new Promise((resolve) => window.setTimeout(resolve, 0));

  assert.deepEqual(writes, ["curl -fsSL moltis.org/install.sh | sh"]);
  assert.equal(button.getAttribute("aria-label"), "Command copied");
  assert.equal(button.querySelector("[data-copy-icon]").classList.contains("hidden"), true);
  assert.equal(button.querySelector("[data-copy-check]").classList.contains("hidden"), false);
  assert.equal(window.document.getElementById("toast").textContent, "Command copied");
  dom.window.close();
});

test("falls back when the Clipboard API is unavailable", async () => {
  const { dom, window } = setup();
  let copiedText = "";
  window.document.execCommand = (command) => {
    copiedText = window.document.querySelector("textarea").value;
    return command === "copy";
  };
  window.eval(clipboardScript);

  const button = window.document.querySelector('#tab-curl button[data-copy-text]');
  button.click();
  await new Promise((resolve) => window.setTimeout(resolve, 0));

  assert.equal(copiedText, "curl -fsSL https://www.moltis.org/install.sh | sh");
  assert.equal(window.document.querySelector("textarea"), null);
  dom.window.close();
});
