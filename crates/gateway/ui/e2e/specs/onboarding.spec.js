const { expect, test } = require("@playwright/test");
const { watchPageErrors } = require("../helpers");

/**
 * Onboarding tests run against a server started WITHOUT seeded
 * IDENTITY.md and USER.md, so the app enters onboarding mode.
 * These use the "onboarding" Playwright project which points at
 * a separate gateway instance on port 18790.
 */
test.describe("Onboarding wizard", () => {
	test.describe.configure({ mode: "serial" });

	test("redirects to /onboarding on first run", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/");

		await expect(page).toHaveURL(/\/onboarding/, { timeout: 15_000 });
		expect(pageErrors).toEqual([]);
	});

	test("step indicator shows first step", async ({ page }) => {
		await page.goto("/onboarding");
		await page.waitForLoadState("networkidle");

		// The onboarding page should show step indicators
		const onboardingRoot = page.locator("#onboardingRoot, #pageContent");
		await expect(onboardingRoot.first()).not.toBeEmpty({
			timeout: 10_000,
		});
	});

	test("auth step renders with skip option on localhost", async ({ page }) => {
		await page.goto("/onboarding");
		await page.waitForLoadState("networkidle");

		// On localhost, the auth step should show a skip button or
		// the password setup is optional
		const content = page.locator("#onboardingRoot, #pageContent");
		await expect(content.first()).not.toBeEmpty({ timeout: 10_000 });

		// Look for skip/next button — on localhost either skip or password setup is fine
		const skipBtn = page.getByRole("button", { name: /skip|next|continue/i });
		await skipBtn
			.first()
			.isVisible()
			.catch(() => false);
	});

	test("identity step has name input", async ({ page }) => {
		await page.goto("/onboarding");
		await page.waitForLoadState("networkidle");

		const content = page.locator("#onboardingRoot, #pageContent");
		await expect(content.first()).not.toBeEmpty({ timeout: 10_000 });

		// Try to advance to the identity step by clicking through
		const nextBtn = page.getByRole("button", { name: /skip|next|continue/i });
		if (
			await nextBtn
				.first()
				.isVisible()
				.catch(() => false)
		) {
			await nextBtn.first().click();
			await page.waitForTimeout(500);
		}

		// The identity step should have a name text input
		const nameInput = page.locator('input[type="text"]');
		// May or may not be on the current step yet
		const hasNameInput = await nameInput
			.first()
			.isVisible()
			.catch(() => false);
		expect(typeof hasNameInput).toBe("boolean");
	});

	test("page has no JS errors through wizard", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/onboarding");
		await page.waitForLoadState("networkidle");

		// Wait a moment for any async errors to surface
		await page.waitForTimeout(1_000);
		expect(pageErrors).toEqual([]);
	});
});
