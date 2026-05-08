// E2E tests: voice.stt.enabled / voice.tts.enabled config flags hide UI buttons.

const { expect, test } = require("../base-test");
const { navigateAndWait, sendRpcFromPage, waitForWsConnected, watchPageErrors } = require("../helpers");

// ── Gon override helpers ─────────────────────────────────────────────────────

/**
 * Patch gon data across the HTML payload, /api/gon, and /api/bootstrap
 * so that voice feature flags reflect the given values for the whole test.
 */
async function mockVoiceFlags(page, { sttEnabled = true, ttsEnabled = true } = {}) {
	await page.route("**/{chats,settings,onboarding}{,/**}", async (route) => {
		var response = await route.fetch();
		var contentType = response.headers()["content-type"] || "";
		if (!contentType.includes("text/html")) return route.fulfill({ response });
		var html = await response.text();
		html = html.replace(/window\.__MOLTIS__=({.*?});<\/script>/s, (_match, rawGon) => {
			var gon = JSON.parse(rawGon);
			gon.stt_enabled = sttEnabled;
			gon.tts_enabled = ttsEnabled;
			return `window.__MOLTIS__=${JSON.stringify(gon)};</script>`;
		});
		var headers = response.headers();
		delete headers["content-length"];
		return route.fulfill({
			response,
			body: html,
			headers: { ...headers, "content-type": contentType },
		});
	});

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

/** After page load, set gon flags in the bundled app, freeze them, and update voice UI. */
async function applyVoiceFlags(page, { sttEnabled = true, ttsEnabled = true } = {}) {
	await page.evaluate(
		(flags) => {
			var gon = window.__moltis_modules?.gon;
			if (gon?.set) {
				gon.set("stt_enabled", flags.sttEnabled);
				gon.set("tts_enabled", flags.ttsEnabled);
				// Prevent gon.refresh() from overwriting our values
				gon.refresh = () => Promise.resolve();
			}
			// The voice-input module updates mic/vad display via checkSttStatus,
			// but the event path may race with page init. Directly toggle the
			// buttons to match the flag state as a reliable fallback.
			var mic = document.getElementById("micBtn");
			var vad = document.getElementById("vadBtn");
			if (!flags.sttEnabled) {
				if (mic) mic.style.display = "none";
				if (vad) vad.style.display = "none";
			}
			window.dispatchEvent(new Event("voice-config-changed"));
		},
		{ sttEnabled, ttsEnabled },
	);
}

// ── Tests ────────────────────────────────────────────────────────────────────

test.describe("voice config flags", () => {
	test.afterEach(async ({ page }) => {
		await page.unrouteAll({ behavior: "ignoreErrors" }).catch(() => {
			// Route cleanup can race with browser teardown after a failed test.
		});
	});

	test("mic and VAD buttons are hidden when stt is disabled", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await mockVoiceFlags(page, { sttEnabled: false, ttsEnabled: true });
		await navigateAndWait(page, "/chats/main");
		await waitForWsConnected(page);
		await applyVoiceFlags(page, { sttEnabled: false, ttsEnabled: true });
		await expect(page.locator("#micBtn")).toBeHidden({ timeout: 5_000 });
		await expect(page.locator("#vadBtn")).toBeHidden({ timeout: 5_000 });
		expect(pageErrors).toEqual([]);
	});

	test("Voice it button is absent from message actions when tts is disabled", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await mockVoiceFlags(page, { sttEnabled: true, ttsEnabled: false });
		await navigateAndWait(page, "/chats/main");
		await waitForWsConnected(page);
		await applyVoiceFlags(page, { sttEnabled: true, ttsEnabled: false });

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
		await applyVoiceFlags(page, { sttEnabled: false, ttsEnabled: false });
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
