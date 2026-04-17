// Tests for GH #729: User messages sent via the GraphQL/RPC API (not the web UI)
// should appear in the web interface in real-time.
//
// The backend now broadcasts a `user_message` event after persisting the user
// message in send_impl().  The frontend handler in websocket.js renders the
// message and caches it (similar to the existing `channel_user` handler),
// skipping rendering when the current connection originated the message
// (the web UI already renders it optimistically via seq-based dedup).

const { expect, test } = require("../base-test");
const { navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

function isRetryableRpcError(message) {
	if (typeof message !== "string") return false;
	return message.includes("WebSocket not connected") || message.includes("WebSocket disconnected");
}

async function sendRpcFromPage(page, method, params) {
	let lastResponse = null;
	for (let attempt = 0; attempt < 40; attempt++) {
		if (attempt > 0) {
			await waitForWsConnected(page);
		}
		lastResponse = await page
			.evaluate(
				async ({ methodName, methodParams }) => {
					var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
					if (!appScript) throw new Error("app module script not found");
					var appUrl = new URL(appScript.src, window.location.origin);
					var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
					var helpers = await import(`${prefix}js/helpers.js`);
					return helpers.sendRpc(methodName, methodParams);
				},
				{
					methodName: method,
					methodParams: params,
				},
			)
			.catch((error) => ({ ok: false, error: { message: error?.message || String(error) } }));

		if (lastResponse?.ok) return lastResponse;
		if (!isRetryableRpcError(lastResponse?.error?.message)) return lastResponse;
	}
	return lastResponse;
}

async function expectRpcOk(page, method, params) {
	const response = await sendRpcFromPage(page, method, params);
	expect(response?.ok, `RPC ${method} failed: ${response?.error?.message || "unknown error"}`).toBeTruthy();
	return response;
}

test.describe("API-sent user messages (GH #729)", () => {
	// Single pageErrors array shared across beforeEach + test body so errors
	// during navigation are not silently dropped.
	let pageErrors;

	test.beforeEach(async ({ page }) => {
		pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/chats/main");
		await waitForWsConnected(page);

		// Wait for session switch and subscription to finish so
		// renderHistory() doesn't clear injected DOM elements and
		// system-event broadcasts are not dropped.
		await page.waitForFunction(
			async () => {
				var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
				if (!appScript) return false;
				var appUrl = new URL(appScript.src, window.location.origin);
				var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
				var state = await import(`${prefix}js/state.js`);
				return state.subscribed && !(state.sessionSwitchInProgress || state.chatBatchLoading);
			},
			{ timeout: 10_000 },
		);
	});

	test("user_message broadcast renders in active session", async ({ page }) => {
		await expectRpcOk(page, "chat.clear", {});

		// Simulate the backend broadcasting a user_message event, as it
		// would after persisting a message sent via the GraphQL API.
		// The backend omits messageIndex (same as channel_user).
		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "main",
				state: "user_message",
				text: "Bonjour Moltis !",
			},
		});

		// The user message should appear in the DOM.
		var userMsg = page.locator(".msg.user");
		await expect(userMsg).toBeVisible({ timeout: 5_000 });
		await expect(userMsg).toContainText("Bonjour Moltis !");

		// It should also be cached in session history.
		const cachedHistory = await page.evaluate(async () => {
			var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			if (!appScript) throw new Error("app module script not found");
			var appUrl = new URL(appScript.src, window.location.origin);
			var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			var cache = await import(`${prefix}js/stores/session-history-cache.js`);
			return cache.getSessionHistory("main");
		});

		expect(cachedHistory).toEqual(
			expect.arrayContaining([
				expect.objectContaining({
					role: "user",
					content: "Bonjour Moltis !",
				}),
			]),
		);
		expect(pageErrors).toEqual([]);
	});

	// Verify the sender's own web UI does not duplicate a message it already
	// rendered optimistically.  The broadcast includes the client seq so the
	// handler can detect "I already rendered this" and suppress the echo.
	test("user_message broadcast is deduplicated for the originating client", async ({ page }) => {
		await expectRpcOk(page, "chat.clear", {});

		// Simulate the web UI having optimistically rendered a message at
		// seq 1: add the DOM element and advance the client chatSeq.
		await page.evaluate(async () => {
			var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			if (!appScript) throw new Error("app module script not found");
			var appUrl = new URL(appScript.src, window.location.origin);
			var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			var chatUi = await import(`${prefix}js/chat-ui.js`);
			var state = await import(`${prefix}js/state.js`);
			var { renderMarkdown } = await import(`${prefix}js/helpers.js`);
			chatUi.chatAddMsg("user", renderMarkdown("Hello from UI"), true);
			state.setChatSeq(1);
		});

		await expect(page.locator(".msg.user")).toHaveCount(1);

		// The backend echoes the same message back with the same seq.
		// The handler should recognise that seq <= chatSeq and skip it.
		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "main",
				state: "user_message",
				text: "Hello from UI",
				seq: 1,
			},
		});

		// Count must stay at 1 — the echo was suppressed.
		await expect(page.locator(".msg.user")).toHaveCount(1, { timeout: 2_000 });
		expect(pageErrors).toEqual([]);
	});

	// Verify that a user_message for a non-active session does not render
	// in the current chat view but does bump the session badge.
	test("user_message for inactive session does not render in active chat", async ({ page }) => {
		await expectRpcOk(page, "chat.clear", {});

		// Broadcast a user_message for a different session.
		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "other-session",
				state: "user_message",
				text: "Message for other session",
			},
		});

		// No user message should appear in the active chat.
		await expect(page.locator(".msg.user")).toHaveCount(0, { timeout: 2_000 });
		expect(pageErrors).toEqual([]);
	});
});
