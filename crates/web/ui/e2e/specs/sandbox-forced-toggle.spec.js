const { expect, test } = require("../base-test");
const { navigateAndWait, waitForWsConnected } = require("../helpers");

// The gateway refuses to take a `sandbox.force` agent out of its sandbox. These
// specs cover the browser half of that: the toggle has to say so and stop being
// a control, without pretending the sandbox is unavailable.

const FORCED_HINT = "This agent's preset sets sandbox.force, so its sandbox cannot be turned off.";
const TOGGLE_HINT = "Toggle sandbox mode";
const RUNTIME_HINT =
	"Sandboxes are disabled on cloud deploys without a container runtime. Install on a VM with Docker or Apple Container to enable this feature.";

/**
 * Pin the backend list the UI fetches on bootstrap.
 *
 * `sandboxRuntimeAvailable()` gates everything else, and it caches the first
 * response, so this has to be installed before the navigation that triggers it.
 * Without it a CI box with no container runtime disables the toggle for an
 * entirely different reason and every assertion below passes for the wrong one.
 */
async function mockSandboxBackends(page, available) {
	await page.route("**/api/sandbox/available-backends", (route) =>
		route.fulfill({
			status: 200,
			contentType: "application/json",
			body: JSON.stringify(
				available
					? { backends: [{ id: "docker", label: "Docker", kind: "container", available: true }], default: "docker" }
					: { backends: [{ id: "none", label: "None", kind: "none", available: false }], default: "none" },
			),
		}),
	);
}

/**
 * Put the chat header into the state a session with the given policy produces,
 * then repaint through the real `updateSandboxUI`.
 *
 * `enabled` is the session's stored `sandbox_enabled`, which is deliberately
 * allowed to disagree with `forced`: a session toggled off before its agent
 * gained `sandbox.force` is exactly the case the effective-state logic exists
 * for.
 */
async function applySessionPolicy(page, { forced, enabled }) {
	await page.evaluate(
		async ([isForced, isEnabled]) => {
			var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			var appUrl = new URL(appScript.src, window.location.origin);
			var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			var S = await import(`${prefix}js/state.js`);
			var sandbox = await import(`${prefix}js/sandbox.js`);
			S.setSessionSandboxForced(isForced);
			sandbox.updateSandboxUI(isEnabled);
		},
		[forced, enabled],
	);
}

/** Record every RPC frame the page sends from now on. */
async function recordSentRpc(page) {
	await page.evaluate(async () => {
		var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
		var appUrl = new URL(appScript.src, window.location.origin);
		var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
		var S = await import(`${prefix}js/state.js`);
		window.__sentRpc = [];
		var socket = S.ws;
		var originalSend = socket.send.bind(socket);
		socket.send = (data) => {
			try {
				window.__sentRpc.push(JSON.parse(data));
			} catch {
				/* non-JSON frames are not RPC */
			}
			return originalSend(data);
		};
	});
}

async function sentMethods(page) {
	return await page.evaluate(() => (window.__sentRpc || []).map((frame) => frame.method));
}

test.describe("forced sandbox toggle", () => {
	test("a forced session pins the sandbox on and disables the toggle", async ({ page }) => {
		await mockSandboxBackends(page, true);
		const pageErrors = await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		// A stored `sandbox_enabled: false` that the gateway now overrules. The
		// label has to follow the effective state, not the stale flag, or the UI
		// tells the operator their commands run on the host while they do not.
		await applySessionPolicy(page, { forced: true, enabled: false });

		const toggle = page.locator("#sandboxToggle");
		await expect(toggle).toBeDisabled();
		await expect(toggle).toHaveAttribute("title", FORCED_HINT);
		await expect(page.locator("#sandboxLabel")).toHaveText("sandboxed");

		expect(pageErrors).toEqual([]);
	});

	test("the forced tooltip names the force policy, not mounts or run_as", async ({ page }) => {
		await mockSandboxBackends(page, true);
		const pageErrors = await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		await applySessionPolicy(page, { forced: true, enabled: true });

		const title = await page.locator("#sandboxToggle").getAttribute("title");
		expect(title).toBe(FORCED_HINT);
		// Mounts and run_as configure the sandbox; only `force` requires one.
		// Naming them here sends an operator to the wrong setting.
		expect(title).not.toMatch(/mounts|run_as/);

		expect(pageErrors).toEqual([]);
	});

	test("a forced toggle stays disabled across a repaint and frees up on an unforced agent", async ({ page }) => {
		await mockSandboxBackends(page, true);
		const pageErrors = await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		const toggle = page.locator("#sandboxToggle");

		await applySessionPolicy(page, { forced: true, enabled: true });
		await expect(toggle).toBeDisabled();

		// Every repaint runs the availability pass again, so a second one must
		// not quietly hand the control back.
		await applySessionPolicy(page, { forced: true, enabled: true });
		await expect(toggle).toBeDisabled();
		await expect(toggle).toHaveAttribute("title", FORCED_HINT);

		// Switching to an agent without the policy is the release condition.
		await applySessionPolicy(page, { forced: false, enabled: true });
		await expect(toggle).toBeEnabled();
		await expect(toggle).toHaveAttribute("title", TOGGLE_HINT);
		await expect(page.locator("#sandboxLabel")).toHaveText("sandboxed");

		// And back again, because agent switching goes both ways.
		await applySessionPolicy(page, { forced: true, enabled: true });
		await expect(toggle).toBeDisabled();

		expect(pageErrors).toEqual([]);
	});

	test("clicking a forced toggle sends no sessions.patch", async ({ page }) => {
		await mockSandboxBackends(page, true);
		const pageErrors = await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		await recordSentRpc(page);
		await applySessionPolicy(page, { forced: true, enabled: true });

		// `disabled` already stops a real user, so drive the handler directly:
		// the in-handler guard is the half that survives a stale repaint, and a
		// patch that leaves here is one the gateway has to refuse.
		await page.locator("#sandboxToggle").dispatchEvent("click");
		await page.waitForTimeout(250);

		expect(await sentMethods(page)).not.toContain("sessions.patch");
		await expect(page.locator("#sandboxLabel")).toHaveText("sandboxed");

		expect(pageErrors).toEqual([]);
	});

	test("an unforced toggle still sends sessions.patch", async ({ page }) => {
		// The control test for the one above: if the guard ever disabled the
		// toggle outright, every assertion there would still pass.
		await mockSandboxBackends(page, true);
		const pageErrors = await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		await recordSentRpc(page);
		await applySessionPolicy(page, { forced: false, enabled: true });

		await page.locator("#sandboxToggle").click();
		await expect.poll(async () => await sentMethods(page), { timeout: 5_000 }).toContain("sessions.patch");

		expect(pageErrors).toEqual([]);
	});

	test("without a container runtime the toggle reports the runtime, not the policy", async ({ page }) => {
		// Both reasons disable the same button. A deploy with no runtime must
		// not be explained as an agent policy the operator could go and change.
		await mockSandboxBackends(page, false);
		const pageErrors = await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		await applySessionPolicy(page, { forced: true, enabled: true });

		const toggle = page.locator("#sandboxToggle");
		await expect(toggle).toBeDisabled();
		await expect(toggle).toHaveAttribute("title", RUNTIME_HINT);
		await expect(page.locator("#sandboxLabel")).toHaveText("direct");

		expect(pageErrors).toEqual([]);
	});
});
