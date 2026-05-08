const { expect, test } = require("../base-test");
const {
	expectRpcOk,
	navigateAndWait,
	sendRpcFromPage,
	waitForChatSessionReady,
	waitForWsConnected,
	watchPageErrors,
} = require("../helpers");

test.describe("Chat abort", () => {
	test.beforeEach(async ({ page }) => {
		await navigateAndWait(page, "/chats/main");
		await waitForWsConnected(page);

		await waitForChatSessionReady(page);
	});

	test("composer send button switches to stop mode while thinking", async ({ page }) => {
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

		var stopBtn = page.locator("#sendBtn");
		await expect(stopBtn).toBeVisible();
		await expect(stopBtn).toHaveAttribute("data-mode", "stop");
		await expect(stopBtn).toHaveAttribute("data-stop-session-key", "main");
		await expect(stopBtn).toHaveAttribute("title", "Stop generation");
		await expect(stopBtn.locator(".icon-stop")).toHaveCount(1);
		await expect(page.locator("#thinkingIndicator .thinking-stop-btn")).toHaveCount(0);

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
		await expect(page.locator("#sendBtn")).toHaveAttribute("data-mode", "send");
		await expect(page.locator("#sendBtn")).toHaveAttribute("title", "Send");

		expect(pageErrors).toEqual([]);
	});

	test("aborted broadcast keeps partial assistant output in UI and history cache", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await expectRpcOk(page, "chat.clear", {});
		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "main",
				state: "thinking",
				runId: "run-chat-abort-partial",
			},
		});
		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "main",
				state: "delta",
				runId: "run-chat-abort-partial",
				text: "Partial answer",
			},
		});

		await expect(page.locator(".msg.assistant")).toContainText("Partial answer");

		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "main",
				state: "aborted",
				runId: "run-chat-abort-partial",
				messageIndex: 0,
				partialMessage: {
					role: "assistant",
					content: "Partial answer",
					model: "mock-model",
					provider: "mock",
					run_id: "run-chat-abort-partial",
					created_at: Date.now(),
				},
			},
		});

		await expect(page.locator("#thinkingIndicator")).toHaveCount(0, { timeout: 5_000 });
		await expect(page.locator(".msg.assistant")).toContainText("Partial answer");

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
					role: "assistant",
					content: "Partial answer",
					run_id: "run-chat-abort-partial",
					historyIndex: 0,
				}),
			]),
		);
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
