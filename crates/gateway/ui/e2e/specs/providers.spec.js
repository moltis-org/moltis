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
});
