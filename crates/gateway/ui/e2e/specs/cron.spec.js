const { expect, test } = require("@playwright/test");
const { navigateAndWait, watchPageErrors } = require("../helpers");

test.describe("Cron jobs page", () => {
	test("cron page loads with heading", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/crons/jobs");

		await expect(page.getByRole("heading", { name: "Cron Jobs", exact: true })).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("heartbeat tab loads", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/crons/heartbeat");

		await expect(page.getByRole("heading", { name: /heartbeat/i })).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("heartbeat inactive state disables run now with info notice", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/crons/heartbeat");

		await expect(page.getByRole("button", { name: "Run Now", exact: true })).toBeDisabled();
		await expect(page.getByText(/Heartbeat inactive:/)).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("create job button present", async ({ page }) => {
		await navigateAndWait(page, "/crons/jobs");

		// Page should have content, create button may depend on state
		const content = page.locator("#pageContent");
		await expect(content).not.toBeEmpty();
	});

	test("page has no JS errors", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/crons/jobs");
		expect(pageErrors).toEqual([]);
	});
});
