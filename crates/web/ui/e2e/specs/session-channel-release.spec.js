// A session attached to a chat (via `/attach`) runs every web turn as an
// untrusted public turn: no tools, no memory, no project context. Releasing the
// binding is the only way back, so the control has to be reachable and has to
// send the clearing patch.

const { expect, test } = require("../base-test");
const { createSession, navigateAndWait, waitForWsConnected } = require("../helpers");

// Capture outgoing RPC payloads without letting them reach the server: the
// server would (correctly) reject clearing a binding this test only faked
// client-side, and what we care about is the request the UI builds.
async function captureRpc(page, method) {
	await page.evaluate(async (watchedMethod) => {
		const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
		if (!appScript) throw new Error("app module script not found");
		const appUrl = new URL(appScript.src, window.location.origin).href;
		const prefix = appUrl.slice(0, appUrl.length - "js/app.js".length);
		const state = await import(`${prefix}js/state.js`);
		const ws = state.ws;
		if (!ws) throw new Error("websocket unavailable");

		window.__capturedRpc = null;
		const originalSend = ws.send.bind(ws);
		ws.send = (payload) => {
			let parsed = null;
			try {
				parsed = JSON.parse(payload);
			} catch (_error) {
				parsed = null;
			}
			if (parsed?.method === watchedMethod) {
				window.__capturedRpc = parsed;
				// Resolve the pending promise so the UI is not left spinning.
				const resolver = state.pending[parsed.id];
				if (resolver) {
					resolver({ ok: true, payload: { channelBinding: null } });
					delete state.pending[parsed.id];
				}
				return;
			}
			originalSend(payload);
		};
	}, method);
}

// SessionHeader is mounted three times (name, toolbar, actions), so every one of
// its controls appears three times. Scope to the actions mount.
function releaseControl(page) {
	return page.locator("#sessionActionsMount").getByTestId("session-unbind-channel");
}

// Mark the active session as attached to a Telegram chat whose own default
// session key differs from this session's key — the shape `/attach` produces.
async function markSessionAttached(page) {
	await expect
		.poll(
			() =>
				page.evaluate(() => {
					const store = window.__moltis_stores?.sessionStore;
					const session = store?.activeSession?.value;
					if (!session) return false;
					session.channelBinding = {
						channel_type: "telegram",
						account_id: "bot1",
						chat_id: "123",
					};
					session.dataVersion.value++;
					return true;
				}),
			{ timeout: 10_000 },
		)
		.toBe(true);
}

test.describe("Releasing a session from its channel", () => {
	test("release control clears the binding via sessions.patch", async ({ page }) => {
		const pageErrors = await navigateAndWait(page, "/");
		await waitForWsConnected(page);
		await createSession(page);
		const sessionKey = await page.evaluate(() => window.__moltis_stores?.sessionStore?.activeSessionKey?.value || "");
		expect(sessionKey).not.toBe("");

		await markSessionAttached(page);

		const release = releaseControl(page);
		await expect(release).toBeVisible({ timeout: 10_000 });
		// The label must say what it restores, not just "unbind".
		await expect(release).toHaveAttribute("title", /without tools or private context/);

		await captureRpc(page, "sessions.patch");
		await release.click();

		await expect
			.poll(() => page.evaluate(() => window.__capturedRpc))
			.toMatchObject({
				method: "sessions.patch",
				params: { key: sessionKey, channelBinding: null },
			});

		expect(pageErrors).toEqual([]);
	});

	// A channel's own session cannot be released — the server refuses it and the
	// next inbound message would re-bind it — so the control must not appear.
	test("a channel's own session offers no release control", async ({ page }) => {
		const pageErrors = await navigateAndWait(page, "/");
		await waitForWsConnected(page);
		await createSession(page);

		await expect
			.poll(
				() =>
					page.evaluate(() => {
						const store = window.__moltis_stores?.sessionStore;
						const session = store?.activeSession?.value;
						if (!session) return false;
						// Binding whose default session key *is* this session's key.
						session.channelBinding = {
							channel_type: "telegram",
							account_id: "bot1",
							chat_id: "123",
						};
						session.key = "telegram:bot1:123";
						session.dataVersion.value++;
						return true;
					}),
				{ timeout: 10_000 },
			)
			.toBe(true);

		await expect(releaseControl(page)).toHaveCount(0);
		expect(pageErrors).toEqual([]);
	});
});
