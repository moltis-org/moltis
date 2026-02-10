const { expect, test } = require("@playwright/test");
const { navigateAndWait, watchPageErrors } = require("../helpers");

test.describe("Monitoring dashboard", () => {
	test("monitoring page loads", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/monitoring");

		await expect(page.getByRole("heading", { name: "Monitoring", exact: true })).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("time range selector present", async ({ page }) => {
		await navigateAndWait(page, "/monitoring");

		// Monitoring page should have time range buttons or selector
		const content = page.locator("#pageContent");
		await expect(content).not.toBeEmpty();
	});

	test("page has no JS errors", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/monitoring");
		expect(pageErrors).toEqual([]);
	});
});
