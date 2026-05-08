const { expect, test } = require("../base-test");
const { navigateAndWait, waitForWsConnected, watchPageErrors, expectRpcOk } = require("../helpers");

async function openVoicePage(page) {
	await navigateAndWait(page, "/settings/voice");
	await expect.poll(() => new URL(page.url()).pathname).toBe("/settings/voice");
}

async function waitForProviderCards(page) {
	await waitForWsConnected(page);
	await expect(page.locator(".provider-card").first()).toBeVisible({ timeout: 15_000 });
}

async function openVoiceTab(page, name) {
	await page.getByRole("tab", { name, exact: true }).click();
	await expect(page.getByRole("tab", { name, exact: true })).toHaveAttribute("aria-selected", "true");
}

/**
 * Find a provider card by its display name text.
 */
function providerCard(page, name) {
	return page
		.locator(".provider-card")
		.filter({ has: page.getByText(name, { exact: true }) })
		.first();
}

/**
 * Open the Configure modal for a provider, fill the API key, and save.
 * Returns true if the modal closed (success), false if the configure
 * button was not visible (provider may already be configured).
 */
async function configureCloudProvider(page, providerName, apiKey) {
	const card = providerCard(page, providerName);
	await expect(card).toBeVisible();

	const configureBtn = card.getByRole("button", { name: "Configure", exact: true });
	if (!(await configureBtn.isVisible().catch(() => false))) return false;

	await configureBtn.click();

	const modal = page
		.locator(".modal-box")
		.filter({ has: page.getByText(providerName, { exact: false }) })
		.last();
	await expect(modal).toBeVisible();

	// Fill API key field — cloud providers show an input with type=password or autocomplete=new-password
	const keyInput = modal.locator('input[type="password"], input[autocomplete="new-password"]').first();
	if (await keyInput.isVisible().catch(() => false)) {
		await keyInput.fill(apiKey);
	}

	await modal.getByRole("button", { name: "Save", exact: true }).click();
	await expect(modal).toBeHidden({ timeout: 10_000 });
	return true;
}

// ── Provider Visibility ─────────────────────────────────────────────────────

test.describe("Voice provider visibility", () => {
	test("voice settings page loads without JS errors", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await openVoicePage(page);
		await waitForProviderCards(page);
		expect(pageErrors).toEqual([]);
	});

	test("all STT providers are visible by default", async ({ page }) => {
		await openVoicePage(page);
		await waitForProviderCards(page);

		const pageText = await page.locator("#pageContent").innerText();

		// Cloud STT
		expect(pageText).toContain("OpenAI Whisper");
		expect(pageText).toContain("Groq");
		expect(pageText).toContain("Deepgram");
		expect(pageText).toContain("Google Cloud STT");
		expect(pageText).toContain("Mistral");
		expect(pageText).toContain("ElevenLabs Scribe");

		// Local STT
		expect(pageText).toContain("Voxtral (Local)");
		expect(pageText).toContain("Whisper (Local)");
		expect(pageText).toContain("whisper.cpp");
		expect(pageText).toContain("sherpa-onnx");
	});

	test("all TTS providers are visible by default", async ({ page }) => {
		await openVoicePage(page);
		await waitForProviderCards(page);
		await openVoiceTab(page, "Text-to-Speech");

		const pageText = await page.locator("#pageContent").innerText();

		expect(pageText).toContain("ElevenLabs");
		expect(pageText).toContain("OpenAI TTS");
		expect(pageText).toContain("Google Cloud TTS");
		expect(pageText).toContain("Piper");
		expect(pageText).toContain("Coqui TTS");
	});
});

// ── Local Provider Instructions ─────────────────────────────────────────────

test.describe("Local provider setup instructions", () => {
	test("whisper-local shows setup instructions", async ({ page }) => {
		await openVoicePage(page);
		await waitForProviderCards(page);

		const card = providerCard(page, "Whisper (Local)");
		await expect(card).toBeVisible();

		await card.getByRole("button", { name: /configure/i }).click();
		const modal = page.locator(".modal-box:visible").last();
		await expect(modal).toContainText("faster-whisper-server", { timeout: 5_000 });
	});

	test("voxtral-local shows setup instructions", async ({ page }) => {
		await openVoicePage(page);
		await waitForProviderCards(page);

		const card = providerCard(page, "Voxtral (Local)");
		await expect(card).toBeVisible();

		await card.getByRole("button", { name: /configure/i }).click();
		const modal = page.locator(".modal-box:visible").last();
		await expect(modal).toContainText(/vllm serve mistralai\/Voxtral/, {
			timeout: 5_000,
		});
	});
});

// ── Cloud STT Provider Configuration ────────────────────────────────────────
//
// These tests require real API keys passed via environment variables.
// They skip gracefully when the key is not set.

const OPENAI_KEY = process.env.MOLTIS_E2E_OPENAI_API_KEY || "";
const GROQ_KEY = process.env.MOLTIS_E2E_GROQ_API_KEY || "";
const DEEPGRAM_KEY = process.env.MOLTIS_E2E_DEEPGRAM_API_KEY || "";
const GOOGLE_KEY = process.env.MOLTIS_E2E_GOOGLE_API_KEY || "";
const MISTRAL_KEY = process.env.MOLTIS_E2E_MISTRAL_API_KEY || "";
const ELEVENLABS_KEY = process.env.MOLTIS_E2E_ELEVENLABS_API_KEY || "";

test.describe("Cloud STT provider configuration", () => {
	test("configure OpenAI Whisper with API key", async ({ page }) => {
		test.skip(!OPENAI_KEY, "requires MOLTIS_E2E_OPENAI_API_KEY");
		const pageErrors = watchPageErrors(page);
		await openVoicePage(page);
		await waitForProviderCards(page);

		await configureCloudProvider(page, "OpenAI Whisper", OPENAI_KEY);

		// After saving, the provider card should show as available
		const card = providerCard(page, "OpenAI Whisper");
		await expect(card).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("configure Groq with API key", async ({ page }) => {
		test.skip(!GROQ_KEY, "requires MOLTIS_E2E_GROQ_API_KEY");
		const pageErrors = watchPageErrors(page);
		await openVoicePage(page);
		await waitForProviderCards(page);

		await configureCloudProvider(page, "Groq", GROQ_KEY);

		const card = providerCard(page, "Groq");
		await expect(card).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("configure Deepgram with API key", async ({ page }) => {
		test.skip(!DEEPGRAM_KEY, "requires MOLTIS_E2E_DEEPGRAM_API_KEY");
		const pageErrors = watchPageErrors(page);
		await openVoicePage(page);
		await waitForProviderCards(page);

		await configureCloudProvider(page, "Deepgram", DEEPGRAM_KEY);

		const card = providerCard(page, "Deepgram");
		await expect(card).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("configure Google Cloud STT with API key", async ({ page }) => {
		test.skip(!GOOGLE_KEY, "requires MOLTIS_E2E_GOOGLE_API_KEY");
		const pageErrors = watchPageErrors(page);
		await openVoicePage(page);
		await waitForProviderCards(page);

		await configureCloudProvider(page, "Google Cloud STT", GOOGLE_KEY);

		const card = providerCard(page, "Google Cloud STT");
		await expect(card).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("configure Mistral (Voxtral) with API key", async ({ page }) => {
		test.skip(!MISTRAL_KEY, "requires MOLTIS_E2E_MISTRAL_API_KEY");
		const pageErrors = watchPageErrors(page);
		await openVoicePage(page);
		await waitForProviderCards(page);

		await configureCloudProvider(page, "Mistral (Voxtral)", MISTRAL_KEY);

		const card = providerCard(page, "Mistral (Voxtral)");
		await expect(card).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("configure ElevenLabs Scribe with API key", async ({ page }) => {
		test.skip(!ELEVENLABS_KEY, "requires MOLTIS_E2E_ELEVENLABS_API_KEY");
		const pageErrors = watchPageErrors(page);
		await openVoicePage(page);
		await waitForProviderCards(page);

		await configureCloudProvider(page, "ElevenLabs Scribe", ELEVENLABS_KEY);

		const card = providerCard(page, "ElevenLabs Scribe");
		await expect(card).toBeVisible();

		expect(pageErrors).toEqual([]);
	});
});

// ── Cloud TTS Provider Configuration ────────────────────────────────────────

test.describe("Cloud TTS provider configuration", () => {
	test("configure OpenAI TTS with API key", async ({ page }) => {
		test.skip(!OPENAI_KEY, "requires MOLTIS_E2E_OPENAI_API_KEY");
		const pageErrors = watchPageErrors(page);
		await openVoicePage(page);
		await waitForProviderCards(page);
		await openVoiceTab(page, "Text-to-Speech");

		await configureCloudProvider(page, "OpenAI TTS", OPENAI_KEY);

		const card = providerCard(page, "OpenAI TTS");
		await expect(card).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("configure ElevenLabs TTS with API key", async ({ page }) => {
		test.skip(!ELEVENLABS_KEY, "requires MOLTIS_E2E_ELEVENLABS_API_KEY");
		const pageErrors = watchPageErrors(page);
		await openVoicePage(page);
		await waitForProviderCards(page);
		await openVoiceTab(page, "Text-to-Speech");

		await configureCloudProvider(page, "ElevenLabs", ELEVENLABS_KEY);

		const card = providerCard(page, "ElevenLabs");
		await expect(card).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("configure Google Cloud TTS with API key", async ({ page }) => {
		test.skip(!GOOGLE_KEY, "requires MOLTIS_E2E_GOOGLE_API_KEY");
		const pageErrors = watchPageErrors(page);
		await openVoicePage(page);
		await waitForProviderCards(page);
		await openVoiceTab(page, "Text-to-Speech");

		await configureCloudProvider(page, "Google Cloud TTS", GOOGLE_KEY);

		const card = providerCard(page, "Google Cloud TTS");
		await expect(card).toBeVisible();

		expect(pageErrors).toEqual([]);
	});
});

// ── Provider Toggle ─────────────────────────────────────────────────────────

test.describe("Voice provider toggle", () => {
	test("toggle OpenAI Whisper on and off via RPC", async ({ page }) => {
		test.skip(!OPENAI_KEY, "requires MOLTIS_E2E_OPENAI_API_KEY");
		const pageErrors = watchPageErrors(page);
		await openVoicePage(page);
		await waitForProviderCards(page);

		// Enable
		await expectRpcOk(page, "voice.provider.toggle", {
			provider: "whisper",
			enabled: true,
			type: "stt",
		});

		// Verify via providers list
		const result = await expectRpcOk(page, "voice.providers.all", {});
		const stt = result?.payload?.stt || [];
		const whisper = stt.find((p) => p.id === "whisper");
		expect(whisper).toBeTruthy();

		// Disable
		await expectRpcOk(page, "voice.provider.toggle", {
			provider: "whisper",
			enabled: false,
			type: "stt",
		});

		expect(pageErrors).toEqual([]);
	});

	test("toggle ElevenLabs TTS on and off via RPC", async ({ page }) => {
		test.skip(!ELEVENLABS_KEY, "requires MOLTIS_E2E_ELEVENLABS_API_KEY");
		const pageErrors = watchPageErrors(page);
		await openVoicePage(page);
		await waitForProviderCards(page);

		await expectRpcOk(page, "voice.provider.toggle", {
			provider: "elevenlabs",
			enabled: true,
			type: "tts",
		});

		await expectRpcOk(page, "voice.provider.toggle", {
			provider: "elevenlabs",
			enabled: false,
			type: "tts",
		});

		expect(pageErrors).toEqual([]);
	});
});
