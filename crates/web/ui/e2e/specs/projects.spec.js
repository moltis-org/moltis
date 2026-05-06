const { expect, test } = require("../base-test");
const { navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

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

	test("projects accessible from settings sidebar", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/projects");

		await expect(page.getByRole("heading", { name: "Repositories", exact: true })).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("page has no JS errors", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/projects");
		expect(pageErrors).toEqual([]);
	});

	test("edit form includes code index checkbox", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/projects");
		await waitForWsConnected(page);

		// Add a project so we have something to edit
		await page.getByPlaceholder("/path/to/project").fill("/tmp/test-project");
		await page.getByRole("button", { name: "Add", exact: true }).click();
		await expect(page.getByText("/tmp/test-project")).toBeVisible({ timeout: 10_000 });

		// Open the edit form for the first project card
		const editButton = page.locator("button").filter({ hasText: /edit/i }).first();
		await expect(editButton).toBeVisible({ timeout: 5_000 });
		await editButton.click();

		// Verify the code index checkbox label is visible
		await expect(page.getByText(/enable code indexing/i)).toBeVisible();

		// Verify the checkbox is present and checked by default
		const checkbox = page.getByRole("checkbox", { name: /enable code indexing/i });
		await expect(checkbox).toBeChecked();

		expect(pageErrors).toEqual([]);
	});
});
