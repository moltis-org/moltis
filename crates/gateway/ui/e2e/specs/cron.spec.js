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

	test("modal defaults are compatible: systemEvent + main", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/crons");

		await page.getByRole("button", { name: "+ Add Job", exact: true }).click();

		await expect(page.locator('[data-field="payloadKind"]')).toHaveValue("systemEvent");
		await expect(page.locator('[data-field="target"]')).toHaveValue("main");
		expect(pageErrors).toEqual([]);
	});

	test("auto-sync: switching payload kind updates session target", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/crons");

		await page.getByRole("button", { name: "+ Add Job", exact: true }).click();

		// Default state
		await expect(page.locator('[data-field="payloadKind"]')).toHaveValue("systemEvent");
		await expect(page.locator('[data-field="target"]')).toHaveValue("main");

		// Switch to agentTurn => target should become isolated
		await page.locator('[data-field="payloadKind"]').selectOption("agentTurn");
		await expect(page.locator('[data-field="target"]')).toHaveValue("isolated");

		// Switch back to systemEvent => target should become main
		await page.locator('[data-field="payloadKind"]').selectOption("systemEvent");
		await expect(page.locator('[data-field="target"]')).toHaveValue("main");

		// Switch target to isolated => payload should become agentTurn
		await page.locator('[data-field="target"]').selectOption("isolated");
		await expect(page.locator('[data-field="payloadKind"]')).toHaveValue("agentTurn");

		// Switch target to main => payload should become systemEvent
		await page.locator('[data-field="target"]').selectOption("main");
		await expect(page.locator('[data-field="payloadKind"]')).toHaveValue("systemEvent");

		expect(pageErrors).toEqual([]);
	});

	test("form fields survive schedule type change", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/crons");

		await page.getByRole("button", { name: "+ Add Job", exact: true }).click();

		// Fill in the name field
		await page.locator('[data-field="name"]').fill("test-job-persist");
		await expect(page.locator('[data-field="name"]')).toHaveValue("test-job-persist");

		// Change schedule type from cron to every
		await page.locator('[data-field="schedKind"]').selectOption("every");

		// Name should still be there
		await expect(page.locator('[data-field="name"]')).toHaveValue("test-job-persist");

		// Change schedule type again to at
		await page.locator('[data-field="schedKind"]').selectOption("at");
		await expect(page.locator('[data-field="name"]')).toHaveValue("test-job-persist");

		expect(pageErrors).toEqual([]);
	});

	test("page has no JS errors", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/crons");
		expect(pageErrors).toEqual([]);
	});
});
