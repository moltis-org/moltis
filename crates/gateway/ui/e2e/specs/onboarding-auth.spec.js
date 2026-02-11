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
		// Fresh runs should land on /onboarding. Retries can land on /login if a
		// previous attempt already configured auth but did not finish onboarding.
		await page.goto("/");
		await expect
			.poll(() => new URL(page.url()).pathname, { timeout: 15_000 })
			.toMatch(/^\/(?:onboarding|login|chats\/.+)$/);

		let pathname = new URL(page.url()).pathname;

		if (pathname === "/login") {
			await page.getByPlaceholder("Enter password").fill("testpassword1");
			await page.getByRole("button", { name: "Sign in", exact: true }).click();
			await expect
				.poll(() => new URL(page.url()).pathname, { timeout: 15_000 })
				.toMatch(/^\/(?:onboarding|chats\/.+)$/);
			pathname = new URL(page.url()).pathname;
		}

		if (/^\/chats\//.test(pathname)) {
			expect(pageErrors).toEqual([]);
			return;
		}

		const authHeading = page.getByRole("heading", { name: "Secure your instance", exact: true });
		const identityHeading = page.getByRole("heading", { name: "Set up your identity", exact: true });
		const providersHeading = page.getByRole("heading", { name: /^(Add LLMs|Add providers)$/ });

		if (await authHeading.isVisible()) {
			await page.getByPlaceholder("6-digit code from terminal").fill("123456");
			await page.locator(".backend-card").filter({ hasText: "Password" }).click();
			const inputs = page.locator("input[type='password']");
			await inputs.first().fill("testpassword1");
			await inputs.nth(1).fill("testpassword1");
			await page.getByRole("button", { name: /^Set password(?: & continue)?$/ }).click();
			await expect(identityHeading).toBeVisible({ timeout: 10_000 });
		} else if (!(await identityHeading.isVisible())) {
			await expect(providersHeading).toBeVisible({ timeout: 10_000 });
			expect(pageErrors).toEqual([]);
			return;
		}

		// Fill identity and save — proves WS is connected (uses sendRpc)
		await page.getByPlaceholder("e.g. Alice").fill("TestUser");
		await page.getByPlaceholder("e.g. Rex").fill("TestBot");
		await page.getByRole("button", { name: "Continue", exact: true }).click();

		// Provider step appears — proves identity save succeeded over WS
		await expect(providersHeading).toBeVisible({ timeout: 10_000 });

		expect(pageErrors).toEqual([]);
	});
});
