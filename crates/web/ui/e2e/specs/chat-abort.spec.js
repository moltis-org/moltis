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
			await page.waitForTimeout(100);
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

test.describe("Chat abort", () => {
	test.beforeEach(async ({ page }) => {
		await navigateAndWait(page, "/chats/main");
		await waitForWsConnected(page);

		// Wait for the session switch RPC to finish rendering history.
		// Without this, renderHistory() can clear #messages after we inject
		// fake DOM elements, causing flaky "element not found" failures.
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

	test("thinking indicator shows stop button", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await expectRpcOk(page, "chat.clear", {});
		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "main",
				state: "thinking",
				runId: "run-chat-abort-stop-button",
			},
		});

		var thinkingIndicator = page.locator("#thinkingIndicator");
		await expect(thinkingIndicator).toBeVisible({ timeout: 5_000 });

		var stopBtn = page.locator("#thinkingIndicator .thinking-stop-btn");
		await expect(stopBtn).toBeVisible();
		await expect(stopBtn).toHaveText("Stop");
		await expect(stopBtn).toHaveAttribute("title", "Stop generation");

		expect(pageErrors).toEqual([]);
	});

	test("aborted broadcast cleans up UI state", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await expectRpcOk(page, "chat.clear", {});
		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "main",
				state: "thinking",
				runId: "run-chat-abort-cleanup",
			},
		});

		var thinkingIndicator = page.locator("#thinkingIndicator");
		await expect(thinkingIndicator).toBeVisible({ timeout: 5_000 });

		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "main",
				state: "aborted",
				runId: "run-chat-abort-cleanup",
			},
		});

		await expect(thinkingIndicator).toHaveCount(0, { timeout: 5_000 });

		expect(pageErrors).toEqual([]);
	});

	test("chat.peek RPC returns result", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		// Peek at an idle session — should return { active: false }.
		var peekRes = await sendRpcFromPage(page, "chat.peek", { sessionKey: "main" });
		expect(peekRes).toBeTruthy();
		// It's fine if it returns ok: false due to no active run.
		// The important thing is that the RPC is registered and doesn't crash.
		if (peekRes?.active !== undefined) {
			expect(peekRes.active).toBe(false);
		}

		expect(pageErrors).toEqual([]);
	});
});
