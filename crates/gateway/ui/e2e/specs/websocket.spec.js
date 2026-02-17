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

	test("final footer shows token speed with slow/fast tones", async ({ page }) => {
		await page.goto("/chats/main");
		await waitForWsConnected(page);
		await expectRpcOk(page, "chat.clear", {});

		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "main",
				state: "final",
				text: "slow reply",
				messageIndex: 999903,
				model: "test-model",
				provider: "test-provider",
				inputTokens: 100,
				outputTokens: 6,
				durationMs: 3000,
				replyMedium: "text",
			},
		});

		const slowAssistant = page.locator("#messages .msg.assistant").last();
		await expect(slowAssistant.locator(".msg-token-speed.msg-token-speed-slow")).toContainText("tok/s");

		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "main",
				state: "final",
				text: "fast reply",
				messageIndex: 999904,
				model: "test-model",
				provider: "test-provider",
				inputTokens: 120,
				outputTokens: 90,
				durationMs: 2000,
				replyMedium: "text",
			},
		});

		const fastAssistant = page.locator("#messages .msg.assistant").last();
		await expect(fastAssistant.locator(".msg-token-speed.msg-token-speed-fast")).toContainText("tok/s");
	});

	test("voice fallback action and warning render for voice final without audio", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/chats/main");
		await waitForWsConnected(page);
		await expectRpcOk(page, "chat.clear", {});

		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "main",
				state: "final",
				text: "voice fallback should be available",
				messageIndex: 999901,
				model: "test-model",
				provider: "test-provider",
				replyMedium: "voice",
				audioWarning: "TTS synthesis failed: timeout",
			},
		});

		var assistant = page.locator("#messages .msg.assistant").last();
		await expect(assistant).toContainText("voice fallback should be available");
		await expect(assistant.locator(".msg-voice-warning")).toContainText("timeout");
		await expect(assistant.locator(".msg-voice-action")).toHaveText("Voice it");
		expect(pageErrors).toEqual([]);
	});

	test("voice fallback action shows error when generation RPC fails", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/chats/main");
		await waitForWsConnected(page);
		await expectRpcOk(page, "chat.clear", {});

		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "main",
				state: "final",
				text: "try generating voice now",
				messageIndex: 999902,
				model: "test-model",
				provider: "test-provider",
				replyMedium: "voice",
			},
		});

		var assistant = page.locator("#messages .msg.assistant").last();
		await expect(assistant.locator(".msg-voice-action")).toHaveText("Voice it");
		await assistant.locator(".msg-voice-action").click();
		await expect(assistant.locator(".msg-voice-action")).toHaveText("Retry voice");
		await expect(assistant.locator(".msg-voice-warning")).not.toHaveText("");
		expect(pageErrors).toEqual([]);
	});

	test("final event is rendered even if switchInProgress gets stuck", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/chats/main");
		await waitForWsConnected(page);
		await expectRpcOk(page, "chat.clear", {});

		await page.evaluate(async () => {
			const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			if (!appScript) throw new Error("app module script not found");
			const appUrl = new URL(appScript.src, window.location.origin);
			const prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			const sessionStoreModule = await import(`${prefix}js/stores/session-store.js`);
			const stateModule = await import(`${prefix}js/state.js`);
			sessionStoreModule.sessionStore.switchInProgress.value = true;
			stateModule.setSessionSwitchInProgress(true);
		});

		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "main",
				state: "final",
				text: "render this final despite stale switch flag",
				messageIndex: 991001,
				model: "test-model",
				provider: "test-provider",
				replyMedium: "text",
				runId: "run-stuck-switch-final",
			},
		});

		await expect(
			page.locator("#messages .msg.assistant").filter({ hasText: "render this final despite stale switch flag" }),
		).toBeVisible();
		await expect
			.poll(() =>
				page.evaluate(async () => {
					const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
					if (!appScript) return null;
					const appUrl = new URL(appScript.src, window.location.origin);
					const prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
					const sessionStoreModule = await import(`${prefix}js/stores/session-store.js`);
					return sessionStoreModule.sessionStore.switchInProgress.value;
				}),
			)
			.toBe(false);

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

	test("map links render place name with right-side rating details", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/chats/main");
		await waitForWsConnected(page);

		await expectRpcOk(page, "chat.clear", {});

		const toolCallId = "map-links-icons-1";
		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "main",
				state: "tool_call_start",
				toolCallId,
				toolName: "show_map",
				arguments: { label: "Tartine Bakery ⭐4.7 - Open till 4PM" },
			},
		});

		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "main",
				state: "tool_call_end",
				toolCallId,
				toolName: "show_map",
				success: true,
				result: {
					label: "Tartine Bakery ⭐4.7 - Open till 4PM",
					map_links: {
						provider: "google_maps",
						url: "https://www.google.com/maps/search/?api=1&query=Tartine+Bakery&center=37.7615,-122.4241",
						google_maps: "https://www.google.com/maps/search/?api=1&query=Tartine+Bakery&center=37.7615,-122.4241",
					},
				},
			},
		});

		const card = page.locator(`#tool-${toolCallId}`);
		await expect(card).toBeVisible();
		await expect(card.locator("img.map-service-icon")).toHaveCount(0);
		const mapLink = card.locator("a.map-link-row");
		await expect(mapLink).toHaveCount(1);
		await expect(mapLink.locator(".map-link-name")).toHaveText("Tartine Bakery");
		await expect(mapLink.locator(".map-link-meta")).toHaveText("⭐4.7 - Open till 4PM");
		await expect(mapLink).toHaveAttribute("title", 'Open "Tartine Bakery ⭐4.7 - Open till 4PM" in maps');
		await expect(card.locator('a:has-text("Tartine Bakery")')).toHaveCount(1);
		expect(pageErrors).toEqual([]);
	});

	test("map links render per-point groups when show_map returns points", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/chats/main");
		await waitForWsConnected(page);

		await expectRpcOk(page, "chat.clear", {});

		const toolCallId = "map-links-points-1";
		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "main",
				state: "tool_call_start",
				toolCallId,
				toolName: "show_map",
				arguments: { label: "Breakfast spots" },
			},
		});

		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "main",
				state: "tool_call_end",
				toolCallId,
				toolName: "show_map",
				success: true,
				result: {
					label: "Breakfast spots",
					map_links: {
						provider: "google_maps",
						url: "https://www.google.com/maps/search/?api=1&query=Breakfast+spots&center=37.788473,-122.408997",
						google_maps: "https://www.google.com/maps/search/?api=1&query=Breakfast+spots&center=37.788473,-122.408997",
					},
					points: [
						{
							label: "Sears Fine Food",
							latitude: 37.788473,
							longitude: -122.408997,
							map_links: {
								provider: "google_maps",
								url: "https://www.google.com/maps/search/?api=1&query=Sears+Fine+Food&center=37.788473,-122.408997",
								google_maps:
									"https://www.google.com/maps/search/?api=1&query=Sears+Fine+Food&center=37.788473,-122.408997",
							},
						},
						{
							label: "Surisan",
							latitude: 37.80895,
							longitude: -122.41576,
							map_links: {
								provider: "google_maps",
								url: "https://www.google.com/maps/search/?api=1&query=Surisan&center=37.80895,-122.41576",
								google_maps: "https://www.google.com/maps/search/?api=1&query=Surisan&center=37.80895,-122.41576",
							},
						},
					],
				},
			},
		});

		const card = page.locator(`#tool-${toolCallId}`);
		await expect(card).toBeVisible();
		await expect(card.locator("img.map-service-icon")).toHaveCount(0);
		await expect(card.locator('a:has-text("Sears Fine Food")')).toHaveCount(1);
		await expect(card.locator('a:has-text("Surisan")')).toHaveCount(1);
		await expect(card.locator('a[title="Open \\"Sears Fine Food\\" in maps"]')).toHaveCount(1);
		await expect(card.locator('a[title="Open \\"Surisan\\" in maps"]')).toHaveCount(1);
		expect(pageErrors).toEqual([]);
	});

	test("thinking text is preserved as reasoning disclosure when tool call follows", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/chats/main");
		await waitForWsConnected(page);

		await expectRpcOk(page, "chat.clear", {});

		// 1. thinking indicator appears
		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: { sessionKey: "main", state: "thinking", runId: "run-think-tool" },
		});
		await expect(page.locator("#thinkingIndicator")).toBeVisible();

		// 2. thinking text arrives
		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "main",
				state: "thinking_text",
				runId: "run-think-tool",
				text: "I need to search the web for recent news",
			},
		});
		await expect(page.locator("#thinkingIndicator .thinking-text")).toContainText("I need to search the web");

		// 3. thinking_done — indicator should NOT be removed yet
		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: { sessionKey: "main", state: "thinking_done", runId: "run-think-tool" },
		});
		await expect(page.locator("#thinkingIndicator")).toBeVisible();

		// 4. tool_call_start — thinking text is preserved as disclosure, indicator removed
		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "main",
				state: "tool_call_start",
				runId: "run-think-tool",
				toolCallId: "tc-web-search-1",
				toolName: "web_search",
				arguments: { query: "top news today" },
			},
		});
		await expect(page.locator("#thinkingIndicator")).toHaveCount(0);
		// Reasoning disclosure is inside the tool card
		const toolCard = page.locator("#tool-run-think-tool-tc-web-search-1");
		await expect(toolCard).toBeVisible();
		await expect(toolCard.locator(".msg-reasoning")).toBeVisible();
		await expect(toolCard.locator(".msg-reasoning-body")).toContainText("I need to search the web for recent news");

		// 5. final with same reasoning should NOT duplicate the disclosure
		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "main",
				state: "final",
				text: "Here are the top news stories.",
				messageIndex: 999998,
				model: "test-model",
				provider: "test-provider",
				replyMedium: "text",
				reasoning: "I need to search the web for recent news",
			},
		});
		// Only one reasoning disclosure should exist (the preserved one, not a duplicate)
		await expect(page.locator(".msg-reasoning")).toHaveCount(1);
		expect(pageErrors).toEqual([]);
	});

	test("whitespace-only streamed assistant bubble is removed once tool call starts/finalizes", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/chats/main");
		await waitForWsConnected(page);
		await expectRpcOk(page, "chat.clear", {});

		// Simulate an assistant stream that emits only whitespace before deciding to call a tool.
		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "main",
				state: "delta",
				runId: "run-whitespace-tool",
				text: " \n\t ",
			},
		});
		await expect(page.locator("#messages .msg.assistant")).toHaveCount(0);

		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "main",
				state: "tool_call_start",
				runId: "run-whitespace-tool",
				toolCallId: "tc-empty-1",
				toolName: "exec",
				arguments: { command: "echo $FOO" },
			},
		});

		const toolCard = page.locator("#tool-run-whitespace-tool-tc-empty-1");
		await expect(toolCard).toBeVisible();
		await expect(page.locator("#messages .msg.assistant")).toHaveCount(0);

		// Final text is also whitespace-only. No empty assistant bubble should be left behind.
		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "main",
				state: "final",
				runId: "run-whitespace-tool",
				text: "\n  \t",
				messageIndex: 999997,
				model: "test-model",
				provider: "test-provider",
				replyMedium: "text",
			},
		});

		await expect(page.locator("#messages .msg.assistant")).toHaveCount(0);
		await expect(toolCard.locator(".msg-model-footer")).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("auth.credentials_changed event redirects through /login", async ({ page }) => {
		await page.goto("/chats/main");
		await waitForWsConnected(page);

		var loginNavigation = page.waitForRequest(
			(request) => request.isNavigationRequest() && new URL(request.url()).pathname === "/login",
			{ timeout: 10_000 },
		);

		// Inject the auth.credentials_changed event via system-event RPC.
		await sendRpcFromPage(page, "system-event", {
			event: "auth.credentials_changed",
			payload: { reason: "test_disconnect" },
		});

		// The event handler should trigger a navigation to /login.
		await loginNavigation;

		// In local no-password mode, /login immediately routes back to chat.
		await expect.poll(() => new URL(page.url()).pathname).toMatch(/^\/(?:login|chats\/.+)$/);
	});
});
