const { expect, test } = require("../base-test");
const { navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

async function openVoicePage(page) {
	await navigateAndWait(page, "/settings/voice");
	await expect.poll(() => new URL(page.url()).pathname).toBe("/settings/voice");
}

test.describe("Voice provider settings", () => {
	test("voice settings page loads without JS errors", async ({ page }) => {
		var pageErrors = watchPageErrors(page);
		await openVoicePage(page);
		await waitForWsConnected(page);
		// Wait for provider cards to render (STT or TTS section)
		await expect(page.locator(".provider-card").first()).toBeVisible({ timeout: 15_000 });
		expect(pageErrors).toEqual([]);
	});

	test("all STT providers are visible by default", async ({ page }) => {
		await openVoicePage(page);
		await waitForWsConnected(page);
		await expect(page.locator(".provider-card").first()).toBeVisible({ timeout: 15_000 });

		var pageText = await page.locator("#pageContent").innerText();

		// Cloud STT providers
		expect(pageText).toContain("OpenAI Whisper");
		expect(pageText).toContain("Groq");
		expect(pageText).toContain("Deepgram");
		expect(pageText).toContain("Google Cloud STT");
		expect(pageText).toContain("Mistral");
		expect(pageText).toContain("ElevenLabs Scribe");

		// Local STT providers
		expect(pageText).toContain("Voxtral (Local)");
		expect(pageText).toContain("Whisper (Local)");
		expect(pageText).toContain("whisper.cpp");
		expect(pageText).toContain("sherpa-onnx");
	});

	test("all TTS providers are visible by default", async ({ page }) => {
		await openVoicePage(page);
		await waitForWsConnected(page);
		await expect(page.locator(".provider-card").first()).toBeVisible({ timeout: 15_000 });

		var pageText = await page.locator("#pageContent").innerText();

		expect(pageText).toContain("ElevenLabs");
		expect(pageText).toContain("OpenAI TTS");
		expect(pageText).toContain("Google Cloud TTS");
		expect(pageText).toContain("Piper");
		expect(pageText).toContain("Coqui TTS");
	});

	test("whisper-local provider shows setup instructions", async ({ page }) => {
		await openVoicePage(page);
		await waitForWsConnected(page);
		await expect(page.locator(".provider-card").first()).toBeVisible({ timeout: 15_000 });

		// Find Whisper (Local) card and click Configure
		var whisperLocalCard = page.locator(".provider-card", { hasText: "Whisper (Local)" });
		await expect(whisperLocalCard).toBeVisible();

		// Click configure button on the card
		var configureBtn = whisperLocalCard.getByRole("button", { name: /configure/i });
		if (await configureBtn.isVisible().catch(() => false)) {
			await configureBtn.click();
			// Should show the setup instructions modal/panel
			await expect(page.getByText("faster-whisper-server")).toBeVisible({ timeout: 5_000 });
		}
	});

	test("voxtral-local provider shows setup instructions", async ({ page }) => {
		await openVoicePage(page);
		await waitForWsConnected(page);
		await expect(page.locator(".provider-card").first()).toBeVisible({ timeout: 15_000 });

		var voxtralCard = page.locator(".provider-card", { hasText: "Voxtral (Local)" });
		await expect(voxtralCard).toBeVisible();

		var configureBtn = voxtralCard.getByRole("button", { name: /configure/i });
		if (await configureBtn.isVisible().catch(() => false)) {
			await configureBtn.click();
			await expect(page.getByText("vllm serve")).toBeVisible({ timeout: 5_000 });
		}
	});
});
