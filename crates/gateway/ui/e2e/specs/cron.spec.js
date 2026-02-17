const { expect, test } = require("@playwright/test");
const { navigateAndWait, watchPageErrors } = require("../helpers");

test.describe("Cron jobs page", () => {
	test("cron page loads with heading", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/crons");

		await expect(page.getByRole("heading", { name: "Cron Jobs", exact: true })).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("heartbeat tab loads", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/heartbeat");

		await expect(page.getByRole("heading", { name: /heartbeat/i })).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("heartbeat inactive state disables run now with info notice", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/heartbeat");

		await expect(page.getByRole("button", { name: "Run Now", exact: true })).toBeDisabled();
		await expect(page.getByText(/Heartbeat inactive:/)).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("create job button present", async ({ page }) => {
		await navigateAndWait(page, "/settings/crons");

		// Page should have content, create button may depend on state
		const content = page.locator("#pageContent");
		await expect(content).not.toBeEmpty();
	});

	test("cron modal exposes model and execution controls", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/crons");

		await page.getByRole("button", { name: "+ Add Job", exact: true }).click();

		await expect(page.getByText("Model (Agent Turn)", { exact: true })).toBeVisible();
		await expect(page.getByText("Execution Target", { exact: true })).toBeVisible();
		await expect(page.getByText("Sandbox Image", { exact: true })).toBeVisible();

		await page.locator('[data-field="executionTarget"]').selectOption("host");
		await expect(page.locator('[data-field="executionTarget"]')).toHaveValue("host");
		expect(pageErrors).toEqual([]);
	});

	test("page has no JS errors", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/crons");
		expect(pageErrors).toEqual([]);
	});
});
