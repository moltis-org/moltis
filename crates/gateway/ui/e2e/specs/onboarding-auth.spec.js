const { expect, test } = require("@playwright/test");
const { watchPageErrors } = require("../helpers");

/**
 * Onboarding tests for remote access (auth required).
 *
 * Uses a gateway started with MOLTIS_BEHIND_PROXY=true (simulates remote)
 * and MOLTIS_E2E_SETUP_CODE=123456 (deterministic setup code).
 * The test verifies that after completing auth, the WebSocket reconnects
 * immediately so subsequent RPC calls (identity save) succeed.
 */
test.describe("Onboarding with forced auth (remote)", () => {
	test.describe.configure({ mode: "serial" });

	test("completes auth and identity steps via WebSocket", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		// Navigate to / — the auth middleware should redirect to /onboarding
		// because MOLTIS_BEHIND_PROXY=true and no password is configured yet.
		await page.goto("/");
		await expect(page).toHaveURL(/\/onboarding/, { timeout: 15_000 });

		// Auth step should be visible (setup code required for remote)
		await expect(page.getByRole("heading", { name: "Secure your instance", exact: true })).toBeVisible();

		// Enter the deterministic setup code
		await page.getByPlaceholder("6-digit code from terminal").fill("123456");

		// Select password method and fill form
		await page.locator(".backend-card").filter({ hasText: "Password" }).click();
		const inputs = page.locator("input[type='password']");
		await inputs.first().fill("testpassword1");
		await inputs.nth(1).fill("testpassword1");

		// Submit — should set cookie and trigger WS reconnect
		await page.getByRole("button", { name: "Set password", exact: true }).click();

		// Identity step appears — proves auth succeeded and step advanced
		await expect(page.getByRole("heading", { name: "Set up your identity", exact: true })).toBeVisible({
			timeout: 10_000,
		});

		// Fill identity and save — proves WS is connected (uses sendRpc)
		await page.getByPlaceholder("e.g. Alice").fill("TestUser");
		await page.getByPlaceholder("e.g. Rex").fill("TestBot");
		await page.getByRole("button", { name: "Continue", exact: true }).click();

		// Provider step appears — proves identity save succeeded over WS
		await expect(page.getByRole("heading", { name: "Add providers", exact: true })).toBeVisible({ timeout: 10_000 });

		expect(pageErrors).toEqual([]);
	});
});
