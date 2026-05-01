// E2E tests: voice.stt.enabled / voice.tts.enabled config flags hide UI buttons.

const { expect, test } = require("../base-test");
const { navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

// ── Gon override helpers ─────────────────────────────────────────────────────

/**
 * Patch gon data across all three layers (initScript, /api/gon, /api/bootstrap)
 * so that voice feature flags reflect the given values for the whole test.
 */
async function mockVoiceFlags(page, { sttEnabled = true, ttsEnabled = true } = {}) {
	await page.addInitScript(
		({ sttEnabled, ttsEnabled }) => {
			var m = window.__MOLTIS__ || {};
			m.stt_enabled = sttEnabled;
			m.tts_enabled = ttsEnabled;
			window.__MOLTIS__ = m;
		},
		{ sttEnabled, ttsEnabled },
	);

	await page.route("**/api/gon*", async (route) => {
		var response = await route.fetch();
		var json = await response.json();
		json.stt_enabled = sttEnabled;
		json.tts_enabled = ttsEnabled;
		return route.fulfill({ response, json });
	});

	await page.route("**/api/bootstrap*", async (route) => {
		var response = await route.fetch();
		var json = await response.json();
		json.stt_enabled = sttEnabled;
		json.tts_enabled = ttsEnabled;
		return route.fulfill({ response, json });
	});
}

// ── RPC helpers (mirrors websocket.spec.js) ──────────────────────────────────

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
				{ methodName: method, methodParams: params },
			)
			.catch((error) => ({ ok: false, error: { message: error?.message || String(error) } }));

		if (lastResponse?.ok) return lastResponse;
		if (!isRetryableRpcError(lastResponse?.error?.message)) return lastResponse;
	}
	return lastResponse;
}

// ── Tests ────────────────────────────────────────────────────────────────────

test.describe("voice config flags", () => {
	test.afterEach(async ({ page }) => {
		await page.unrouteAll({ behavior: "ignoreErrors" }).catch(() => {});
	});

	test("mic and VAD buttons are hidden when stt is disabled", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await mockVoiceFlags(page, { sttEnabled: false, ttsEnabled: true });
		await navigateAndWait(page, "/chats/main");
		await waitForWsConnected(page);

		await expect(page.locator("#micBtn")).toBeHidden({ timeout: 5_000 });
		await expect(page.locator("#vadBtn")).toBeHidden({ timeout: 5_000 });
		expect(pageErrors).toEqual([]);
	});

	test("Voice it button is absent from message actions when tts is disabled", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await mockVoiceFlags(page, { sttEnabled: true, ttsEnabled: false });
		await navigateAndWait(page, "/chats/main");
		await waitForWsConnected(page);

		// Inject a regular assistant message with text (no audio).
		await sendRpcFromPage(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "main",
				state: "final",
				text: "tts flag test message",
				messageIndex: 999920,
				model: "test-model",
				provider: "test-provider",
			},
		});

		const assistant = page.locator("#messages .msg.assistant").last();
		await expect(assistant).toContainText("tts flag test message", { timeout: 5_000 });

		// Action bar should exist but must not contain a "Voice it" button.
		await expect(assistant.locator('.msg-action-btn[title="Voice it"]')).toHaveCount(0);
		expect(pageErrors).toEqual([]);
	});

	test("Voice it button is present in message actions when tts is enabled", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await mockVoiceFlags(page, { sttEnabled: true, ttsEnabled: true });
		await navigateAndWait(page, "/chats/main");
		await waitForWsConnected(page);

		await sendRpcFromPage(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "main",
				state: "final",
				text: "tts flag enabled test message",
				messageIndex: 999921,
				model: "test-model",
				provider: "test-provider",
			},
		});

		const assistant = page.locator("#messages .msg.assistant").last();
		await expect(assistant).toContainText("tts flag enabled test message", { timeout: 5_000 });
		await expect(assistant.locator('.msg-action-btn[title="Voice it"]')).toHaveCount(1);
		expect(pageErrors).toEqual([]);
	});

	test("both mic/VAD and Voice it are hidden when both stt and tts are disabled", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await mockVoiceFlags(page, { sttEnabled: false, ttsEnabled: false });
		await navigateAndWait(page, "/chats/main");
		await waitForWsConnected(page);

		await expect(page.locator("#micBtn")).toBeHidden({ timeout: 5_000 });
		await expect(page.locator("#vadBtn")).toBeHidden({ timeout: 5_000 });

		await sendRpcFromPage(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: "main",
				state: "final",
				text: "both flags disabled test message",
				messageIndex: 999922,
				model: "test-model",
				provider: "test-provider",
			},
		});

		const assistant = page.locator("#messages .msg.assistant").last();
		await expect(assistant).toContainText("both flags disabled test message", { timeout: 5_000 });
		await expect(assistant.locator('.msg-action-btn[title="Voice it"]')).toHaveCount(0);
		expect(pageErrors).toEqual([]);
	});
});
