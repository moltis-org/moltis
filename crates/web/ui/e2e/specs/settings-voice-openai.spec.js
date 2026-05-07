const { expect, test } = require("../base-test");
const { navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

async function mockVoiceProviders(page) {
	await expect.poll(() => new URL(page.url()).pathname).toBe("/settings/voice");
	await expect(page.getByRole("heading", { name: "Voice", exact: true })).toBeVisible();
	await page.waitForFunction(() => !!document.querySelector('script[type="module"][src*="js/app.js"]'));
	await page.evaluate(async () => {
		const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
		if (!appScript) throw new Error("app.js script not found");
		const appUrl = new URL(appScript.src, window.location.origin).href;
		const marker = "js/app.js";
		const markerIdx = appUrl.indexOf(marker);
		if (markerIdx < 0) throw new Error("app.js marker not found in script URL");
		const prefix = appUrl.slice(0, markerIdx);
		const state = await import(`${prefix}js/state.js`);
		const wsOpen = typeof WebSocket !== "undefined" ? WebSocket.OPEN : 1;
		window.__voiceSettingsRequests = [];
		const providers = {
			stt: [
				{
					id: "whisper",
					name: "OpenAI Whisper",
					type: "stt",
					category: "cloud",
					description: "OpenAI clip transcription. Realtime voice models require the Realtime API.",
					available: true,
					enabled: true,
					preferred: true,
					keySource: "env",
					keyPlaceholder: "sk-...",
					keyUrl: "https://platform.openai.com/api-keys",
					keyUrlLabel: "platform.openai.com/api-keys",
					hint: "gpt-realtime-2, gpt-realtime-translate, and gpt-realtime-whisper are Realtime API models. Moltis currently records a clip and uses OpenAI's transcription endpoint for this provider.",
					capabilities: {
						baseUrl: true,
						customModel: true,
						modelChoices: ["whisper-1", "gpt-4o-transcribe", "gpt-4o-mini-transcribe"],
						realtimeModelChoices: ["gpt-realtime-2", "gpt-realtime-translate", "gpt-realtime-whisper"],
					},
					settings: {
						baseUrl: null,
						model: "whisper-1",
					},
					settingsSummary: "whisper-1",
				},
			],
			tts: [],
		};
		state.setConnected(false);
		function respond(req) {
			if (req.method === "voice.providers.all") return { ok: true, payload: providers };
			if (req.method === "voice.config.voxtral_requirements") {
				return { ok: true, payload: { os: "macos", arch: "aarch64", compatible: false, reasons: [] } };
			}
			if (req.method === "voice.config.save_settings") return { ok: true, payload: { ok: true } };
			if (req.method === "voice.provider.toggle") return { ok: true, payload: { ok: true } };
			if (req.method === "voice.personas.list") return { ok: true, payload: { personas: [] } };
			return {
				ok: false,
				error: { message: `unexpected rpc in voice settings test: ${req.method}` },
			};
		}

		state.setWs({
			readyState: wsOpen,
			send(raw) {
				const req = JSON.parse(raw || "{}");
				const resolver = state.pending[req.id];
				if (!resolver) return;
				window.__voiceSettingsRequests.push({
					method: req.method,
					params: req.params || {},
				});
				resolver(respond(req));
				delete state.pending[req.id];
			},
		});
		state.setConnected(true);
	});
}

test.describe("Settings > Voice > OpenAI", () => {
	test("saves OpenAI transcription model and shows Realtime model guidance", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/voice");
		await waitForWsConnected(page);
		await mockVoiceProviders(page);

		const whisperRow = page
			.locator(".provider-card")
			.filter({ has: page.getByText("OpenAI Whisper", { exact: true }) });
		await expect(whisperRow).toContainText("Realtime voice models require the Realtime API");
		await whisperRow.getByRole("button", { name: "Configure", exact: true }).click();

		const modal = page.locator(".modal-box").filter({ has: page.getByText("Add OpenAI Whisper", { exact: true }) });
		await expect(modal).toBeVisible();
		await expect(modal).toContainText("gpt-realtime-2");
		await expect(modal).toContainText("gpt-realtime-translate");
		await expect(modal).toContainText("gpt-realtime-whisper");

		await modal.locator('input[placeholder="model (optional)"]').fill("gpt-4o-mini-transcribe");
		await modal.getByRole("button", { name: "Save", exact: true }).click();

		await expect
			.poll(() =>
				page.evaluate(
					() => window.__voiceSettingsRequests.find((req) => req.method === "voice.config.save_settings") || null,
				),
			)
			.not.toBeNull();
		const saveRequest = await page.evaluate(() =>
			window.__voiceSettingsRequests.find((req) => req.method === "voice.config.save_settings"),
		);
		expect(saveRequest.params).toMatchObject({
			provider: "whisper",
			model: "gpt-4o-mini-transcribe",
		});
		expect(pageErrors).toEqual([]);
	});
});
