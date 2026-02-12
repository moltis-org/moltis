const { expect, test } = require("@playwright/test");
const { waitForWsConnected, watchPageErrors } = require("../helpers");

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

test.describe("WebSocket connection lifecycle", () => {
	test("status shows connected after page load", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/");
		await waitForWsConnected(page);

		await expect(page.locator("#statusDot")).toHaveClass(/connected/);
		// When connected, statusText is intentionally cleared to ""
		await expect(page.locator("#statusText")).toHaveText("");
		expect(pageErrors).toEqual([]);
	});

	test("memory info updates from tick events", async ({ page }) => {
		await page.goto("/");
		await waitForWsConnected(page);

		// tick events carry memory stats; wait for memoryInfo to populate
		await expect(page.locator("#memoryInfo")).not.toHaveText("", {
			timeout: 15_000,
		});
	});

	test("connection persists across SPA navigation", async ({ page }) => {
		await page.goto("/");
		await waitForWsConnected(page);

		// Navigate to a different page within the SPA
		await page.goto("/settings");
		await expect(page.locator("#pageContent")).not.toBeEmpty();

		// WebSocket should remain connected through client-side navigation
		await expect(page.locator("#statusDot")).toHaveClass(/connected/);

		// Navigate back to chat
		await page.goto("/chats/main");
		await expect(page.locator("#pageContent")).not.toBeEmpty();
		await expect(page.locator("#statusDot")).toHaveClass(/connected/);
	});

	test("health endpoint responds", async ({ request }) => {
		// Verify the server is healthy via the HTTP health endpoint
		const resp = await request.get("/health");
		expect(resp.ok()).toBeTruthy();
	});

	test("final chat text is kept when it includes tool output plus analysis", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/chats/main");
		await waitForWsConnected(page);

		await expectRpcOk(page, "chat.clear", {});

		const toolOutput = "Linux moltis-moltis-sandbox-main 6.12.28 #1 SMP Tue May 20 15:19:05 UTC 2025 aarch64 GNU/Linux";
		const finalText =
			"The command executed successfully. The output shows:\n- Kernel name: Linux\n- Hostname: moltis-moltis-sandbox-main\n\n" +
			toolOutput;

		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "main",
				state: "tool_call_end",
				toolCallId: "echo-test",
				success: true,
				result: { stdout: toolOutput, stderr: "", exit_code: 0 },
			},
		});

		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "main",
				state: "delta",
				text: finalText,
			},
		});

		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "main",
				state: "final",
				text: finalText,
				messageIndex: 999,
				model: "test-model",
				provider: "test-provider",
				replyMedium: "text",
			},
		});

		await expect(
			page.locator("#messages .msg.assistant").filter({ hasText: "command executed successfully" }),
		).toBeVisible();
		await expect(
			page.locator("#messages .msg.assistant").filter({ hasText: "moltis-moltis-sandbox-main" }),
		).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("out-of-order tool events still resolve exec card", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/chats/main");
		await waitForWsConnected(page);

		await expectRpcOk(page, "chat.clear", {});

		const toolCallId = "reorder-exec-1";
		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "main",
				state: "tool_call_end",
				toolCallId,
				toolName: "exec",
				success: true,
				result: { stdout: "ok", stderr: "", exit_code: 0 },
			},
		});

		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "main",
				state: "tool_call_start",
				toolCallId,
				toolName: "exec",
				arguments: { command: "df -h" },
			},
		});

		const card = page.locator(`#tool-${toolCallId}`);
		await expect(card).toBeVisible();
		await expect(card).toHaveClass(/exec-ok/);
		await expect(page.locator(`#tool-${toolCallId} .exec-status`)).toHaveCount(0);
		await expect(page.locator(`#tool-${toolCallId} .exec-output`)).toContainText("ok");
		expect(pageErrors).toEqual([]);
	});

	test("final event clears stale running exec status when tool end is missed", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/chats/main");
		await waitForWsConnected(page);

		await expectRpcOk(page, "chat.clear", {});

		const toolCallId = "stale-exec-1";
		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "main",
				state: "tool_call_start",
				toolCallId,
				toolName: "exec",
				arguments: { command: "df -h" },
			},
		});

		await expect(page.locator(`#tool-${toolCallId} .exec-status`)).toBeVisible();

		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "main",
				state: "final",
				text: "done",
				messageIndex: 999999,
				model: "test-model",
				provider: "test-provider",
				replyMedium: "text",
			},
		});

		await expect(page.locator(`#tool-${toolCallId} .exec-status`)).toHaveCount(0);
		await expect(page.locator(`#tool-${toolCallId}`)).toHaveClass(/exec-ok/);
		expect(pageErrors).toEqual([]);
	});

	test("auth.credentials_changed event redirects to /login", async ({ page }) => {
		await page.goto("/chats/main");
		await waitForWsConnected(page);

		// Inject the auth.credentials_changed event via system-event RPC.
		await sendRpcFromPage(page, "system-event", {
			event: "auth.credentials_changed",
			payload: { reason: "test_disconnect" },
		});

		// The event handler should redirect the browser to /login.
		await expect.poll(() => new URL(page.url()).pathname).toBe("/login");
	});
});
