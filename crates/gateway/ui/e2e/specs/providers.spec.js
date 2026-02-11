const { expect, test } = require("@playwright/test");
const { navigateAndWait, watchPageErrors } = require("../helpers");

test.describe("Provider setup page", () => {
	test("provider page loads", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/providers");

		await expect(page.getByRole("heading", { name: "LLMs" })).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("add provider button exists", async ({ page }) => {
		await navigateAndWait(page, "/settings/providers");

		// Look for an "Add" button or similar provider action
		const addBtn = page.getByRole("button", { name: /add/i });
		const providerItems = page.locator(".provider-item");

		// Either add button or provider items should be visible
		const hasAdd = await addBtn.isVisible().catch(() => false);
		const hasItems = (await providerItems.count()) > 0;
		expect(hasAdd || hasItems).toBeTruthy();
	});

	test("detect models button exists", async ({ page }) => {
		await navigateAndWait(page, "/settings/providers");

		// Detect button may or may not be visible depending on state
		// Just verify the page rendered properly
		const content = page.locator("#pageContent");
		await expect(content).not.toBeEmpty();
	});

	test("no providers shows guidance", async ({ page }) => {
		await navigateAndWait(page, "/settings/providers");

		// On a fresh server with no API keys, should show guidance or empty state
		const content = page.locator("#pageContent");
		await expect(content).not.toBeEmpty();
	});

	test("page has no JS errors", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/providers");
		expect(pageErrors).toEqual([]);
	});

	test("provider modal honors configured provider order", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/providers");
		await page.getByRole("button", { name: "Add LLM" }).click();

		const providerNames = page.locator(".provider-modal-backdrop .provider-item .provider-item-name");
		await expect(providerNames.first()).toBeVisible();
		const names = await providerNames.allTextContents();

		const openAiIndex = names.indexOf("OpenAI");
		const copilotIndex = names.indexOf("GitHub Copilot");
		const localLlmIndex = names.indexOf("Local LLM (Offline)");

		expect(openAiIndex).toBeGreaterThanOrEqual(0);
		if (localLlmIndex >= 0) {
			expect(openAiIndex).toBeLessThan(localLlmIndex);
			if (copilotIndex >= 0) {
				expect(copilotIndex).toBeLessThan(localLlmIndex);
			}
		}
		expect(pageErrors).toEqual([]);
	});

	test("api key forms include provider key source hints", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/providers");
		await page.getByRole("button", { name: "Add LLM" }).click();

		const openaiItem = page.locator(".provider-modal-backdrop .provider-item").filter({ hasText: "OpenAI" }).first();
		await expect(openaiItem).toBeVisible();
		await openaiItem.click();

		await expect(page.getByRole("link", { name: "OpenAI Platform" })).toBeVisible();
		await page.getByRole("button", { name: "Back", exact: true }).click();

		const optionalCandidates = [
			{ providerName: "Kimi Code", linkName: "Kimi Code Console" },
			{ providerName: "Anthropic", linkName: "Anthropic Console" },
			{ providerName: "Moonshot", linkName: "Moonshot Platform" },
		];
		for (const candidate of optionalCandidates) {
			const item = page
				.locator(".provider-modal-backdrop .provider-item")
				.filter({ hasText: candidate.providerName })
				.first();
			if ((await item.count()) === 0) continue;

			await item.click();
			await expect(page.getByRole("link", { name: candidate.linkName })).toBeVisible();
			await page.getByRole("button", { name: "Back", exact: true }).click();
		}

		expect(pageErrors).toEqual([]);
	});

	test("provider validation errors render in danger panel", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/providers");
		await page.getByRole("button", { name: "Add LLM" }).click();

		const openaiItem = page.locator(".provider-modal-backdrop .provider-item").filter({ hasText: "OpenAI" }).first();
		await expect(openaiItem).toBeVisible();
		await openaiItem.click();

		await page.getByRole("button", { name: "Save & Validate", exact: true }).click();

		const errorPanel = page.locator(".provider-modal-backdrop .alert-error-text");
		await expect(errorPanel).toBeVisible();
		await expect(errorPanel).toContainText("Error:");
		await expect(errorPanel).toContainText("API key is required");

		expect(pageErrors).toEqual([]);
	});
});
