const { expect, test } = require("../base-test");
const { navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

test.describe("Telephony channel", () => {
	test("connect button visible when telephony is offered", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const addButton = page.getByRole("button", { name: "Connect Phone Calls", exact: true });
		await expect(addButton).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("add modal opens with correct title", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		await page.getByRole("button", { name: "Connect Phone Calls", exact: true }).click();
		await expect(
			page.getByRole("heading", { name: "Connect Phone Calls", exact: true }),
		).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("add modal has Twilio credential fields", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		await page.getByRole("button", { name: "Connect Phone Calls", exact: true }).click();
		await expect(
			page.getByRole("heading", { name: "Connect Phone Calls", exact: true }),
		).toBeVisible();

		// Account SID field
		const sidInput = page.locator('input[placeholder="AC..."]');
		await expect(sidInput).toBeVisible();

		// Auth Token field
		const tokenInput = page.locator('input[type="password"][placeholder="Auth token"]');
		await expect(tokenInput).toBeVisible();

		// Phone number field
		const phoneInput = page.locator('input[placeholder="+15551234567"]');
		await expect(phoneInput).toBeVisible();

		// Webhook URL field
		const webhookInput = page.locator('input[placeholder="https://your-domain.com"]');
		await expect(webhookInput).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("add modal has inbound policy select", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		await page.getByRole("button", { name: "Connect Phone Calls", exact: true }).click();

		// Inbound policy select defaults to "disabled"
		const policySelect = page.locator("select").filter({ hasText: "Disabled" });
		await expect(policySelect).toBeVisible();

		// Should have the three options
		const options = policySelect.locator("option");
		await expect(options).toHaveCount(3);

		expect(pageErrors).toEqual([]);
	});

	test("add modal shows Twilio help text", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		await page.getByRole("button", { name: "Connect Phone Calls", exact: true }).click();

		await expect(page.getByText("Twilio Phone Calls")).toBeVisible();
		await expect(page.getByText("Twilio account")).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("add modal validates required fields", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		await page.getByRole("button", { name: "Connect Phone Calls", exact: true }).click();
		await expect(
			page.getByRole("heading", { name: "Connect Phone Calls", exact: true }),
		).toBeVisible();

		// Click submit without filling anything
		await page.getByRole("button", { name: "Connect Phone Calls", exact: false }).last().click();

		// Should show error about missing Account SID
		await expect(page.getByText("Account SID is required")).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("add modal validates E.164 phone format", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		await page.getByRole("button", { name: "Connect Phone Calls", exact: true }).click();

		// Fill SID and token but bad phone number
		await page.locator('input[placeholder="AC..."]').fill("AC_TEST_SID");
		await page.locator('input[type="password"][placeholder="Auth token"]').fill("test_token");
		await page.locator('input[placeholder="+15551234567"]').fill("5551234567");

		await page.getByRole("button", { name: "Connect Phone Calls", exact: false }).last().click();

		// Should show E.164 format error
		await expect(page.getByText("E.164 format")).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("add modal has model select and connect button", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		await page.getByRole("button", { name: "Connect Phone Calls", exact: true }).click();

		// Model select should be present
		await expect(page.getByText("Default Model")).toBeVisible();

		// Submit button should be present
		const submitButton = page
			.getByRole("button", { name: "Connect Phone Calls", exact: false })
			.last();
		await expect(submitButton).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("add modal shows allowlist when policy is allowlist", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		await page.getByRole("button", { name: "Connect Phone Calls", exact: true }).click();

		// Initially allowlist should be hidden (policy=disabled)
		await expect(page.getByText("Allowed Callers")).not.toBeVisible();

		// Change policy to allowlist
		const policySelect = page.locator("select").filter({ hasText: "Disabled" });
		await policySelect.selectOption("allowlist");

		// Now allowlist should be visible
		await expect(page.getByText("Allowed Callers")).toBeVisible();

		expect(pageErrors).toEqual([]);
	});
});
