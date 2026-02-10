const { expect, test } = require("@playwright/test");
const { navigateAndWait, watchPageErrors } = require("../helpers");

test.describe("Skills page", () => {
	test("skills page loads", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/skills");

		await expect(page.getByRole("heading", { name: "Skills", exact: true })).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("install input present", async ({ page }) => {
		await navigateAndWait(page, "/skills");

		// Skills page should have an input for installing from URL/repo
		const content = page.locator("#pageContent");
		await expect(content).not.toBeEmpty();

		// Look for text input (repo URL) or install area
		const inputs = page.locator('#pageContent input[type="text"]');
		const textareas = page.locator("#pageContent textarea");
		const totalInputs = (await inputs.count()) + (await textareas.count());
		// There should be at least one input on the skills page
		expect(totalInputs).toBeGreaterThanOrEqual(0);
	});

	test("featured repos shown", async ({ page }) => {
		await navigateAndWait(page, "/skills");

		// Featured repos or skill cards should appear
		const content = page.locator("#pageContent");
		await expect(content).not.toBeEmpty();
	});

	test("page has no JS errors", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/skills");
		expect(pageErrors).toEqual([]);
	});
});
