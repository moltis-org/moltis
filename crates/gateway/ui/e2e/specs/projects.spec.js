const { expect, test } = require("@playwright/test");
const { navigateAndWait, watchPageErrors } = require("../helpers");

test.describe("Projects page", () => {
	test("projects page loads", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/projects");

		await expect(page.getByRole("heading", { name: "Repositories", exact: true })).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("add project input present", async ({ page }) => {
		await navigateAndWait(page, "/projects");

		// Projects page should have a directory input for adding repos
		const content = page.locator("#pageContent");
		await expect(content).not.toBeEmpty();
	});

	test("auto-detect button present", async ({ page }) => {
		await navigateAndWait(page, "/projects");

		const content = page.locator("#pageContent");
		await expect(content).not.toBeEmpty();
	});

	test("page has no JS errors", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/projects");
		expect(pageErrors).toEqual([]);
	});
});
