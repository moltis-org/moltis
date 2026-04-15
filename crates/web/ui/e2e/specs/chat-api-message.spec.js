// Tests for GH #729: User messages sent via the GraphQL/RPC API (not the web UI)
// should appear in the web interface in real-time.
//
// Currently, when a message is sent via `chat.send` from an external client
// (GraphQL mutation, mobile app, etc.), the backend persists it and runs the
// LLM, but no WebSocket event is broadcast for the user message itself.  The
// web UI only shows it after a full page reload or session switch.
//
// The fix will:
//  1. Backend: broadcast a `user_message` event after persisting the user
//     message in send_impl().
//  2. Frontend: add a `user_message` handler in websocket.js that renders the
//     message and caches it, similar to the existing `channel_user` handler.
//  3. Frontend: skip rendering when the current connection originated the
//     message (the web UI already renders it optimistically).

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
	test.beforeEach(async ({ page }) => {
		await navigateAndWait(page, "/chats/main");
		await waitForWsConnected(page);

		// Wait for session switch to finish so renderHistory() doesn't
		// clear injected DOM elements.
		await page.waitForFunction(
			async () => {
				var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
				if (!appScript) return false;
				var appUrl = new URL(appScript.src, window.location.origin);
				var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
				var state = await import(`${prefix}js/state.js`);
				return !(state.sessionSwitchInProgress || state.chatBatchLoading);
			},
			{ timeout: 10_000 },
		);
	});

	// This test verifies the fix for GH #729: once the backend broadcasts a
	// `user_message` event and the frontend handles it, an API-sent message
	// should appear in the chat without a page reload.
	test.fixme("user_message broadcast renders in active session", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await expectRpcOk(page, "chat.clear", {});

		// Simulate the backend broadcasting a user_message event, as it
		// would after persisting a message sent via the GraphQL API.
		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "main",
				state: "user_message",
				text: "Bonjour Moltis !",
				messageIndex: 0,
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
	// rendered optimistically.  The broadcast should include the client seq
	// so the frontend can detect "I already rendered this".
	test.fixme("user_message broadcast is deduplicated for the originating client", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await expectRpcOk(page, "chat.clear", {});

		// Simulate the web UI having already rendered this message
		// optimistically (seq 1).
		await page.evaluate(async () => {
			var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			if (!appScript) throw new Error("app module script not found");
			var appUrl = new URL(appScript.src, window.location.origin);
			var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			var chatUi = await import(`${prefix}js/chat-ui.js`);
			var { renderMarkdown } = await import(`${prefix}js/helpers.js`);
			chatUi.chatAddMsg("user", renderMarkdown("Already rendered"), true);
		});

		await expect(page.locator(".msg.user")).toHaveCount(1);

		// Now the broadcast arrives — same content but different origin.
		// The frontend should add it because it came from another client.
		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "main",
				state: "user_message",
				text: "From API client",
				messageIndex: 1,
			},
		});

		// Should now have two user messages (the local one + the API one).
		await expect(page.locator(".msg.user")).toHaveCount(2, { timeout: 5_000 });
		expect(pageErrors).toEqual([]);
	});

	// Verify that a user_message for a non-active session does not render
	// in the current chat view but does bump the session badge.
	test.fixme("user_message for inactive session does not render in active chat", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await expectRpcOk(page, "chat.clear", {});

		// Broadcast a user_message for a different session.
		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "other-session",
				state: "user_message",
				text: "Message for other session",
				messageIndex: 0,
			},
		});

		// No user message should appear in the active chat.
		await expect(page.locator(".msg.user")).toHaveCount(0, { timeout: 2_000 });
		expect(pageErrors).toEqual([]);
	});
});
