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

		await expect(page.getByText("Directory", { exact: true })).toBeVisible();
		await expect(page.getByPlaceholder("/path/to/project")).toBeVisible();
		await expect(page.getByRole("button", { name: "Add", exact: true })).toBeVisible();
	});

	test("auto-detect button present", async ({ page }) => {
		await navigateAndWait(page, "/projects");

		await expect(page.getByRole("button", { name: "Auto-detect", exact: true })).toBeVisible();
		await expect(page.getByRole("button", { name: "Clear All", exact: true })).toBeVisible();
		await expect(page.getByText(/does not delete anything from disk/i)).toBeVisible();
		await expect(page.getByText(/scans common directories/i)).toBeVisible();
	});

	test("projects route is hidden from nav", async ({ page }) => {
		await navigateAndWait(page, "/projects");
		await expect(page.locator('a.nav-link[href="/projects"]')).toHaveCount(0);
	});

	test("page has no JS errors", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/projects");
		expect(pageErrors).toEqual([]);
	});
});
