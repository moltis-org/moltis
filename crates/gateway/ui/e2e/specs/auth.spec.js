const { expect, test } = require("@playwright/test");
const { expectPageContentMounted, watchPageErrors } = require("../helpers");

/**
 * Auth tests verify authentication behavior on the shared server.
 *
 * Since the test server runs on localhost with seeded identity (no password),
 * auth is bypassed. These tests verify that bypass behavior and the auth
 * status API. Setting a password requires a setup code printed to the
 * server's terminal, which is not capturable from Playwright — so
 * password/login flow tests are deferred to manual QA or a dedicated
 * test harness.
 */
test.describe("Authentication", () => {
	test("auth status API returns expected state on localhost", async ({ request }) => {
		const resp = await request.get("/api/auth/status");
		expect(resp.ok()).toBeTruthy();

		const data = await resp.json();
		// On localhost with no password set, auth is bypassed
		expect(data.authenticated).toBeTruthy();
		expect(data.setup_required).toBeFalsy();
	});

	test("pages load without login prompt on localhost", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/");

		// Should NOT redirect to /login since auth is bypassed on localhost
		await expect(page).toHaveURL(/\/chats\//);
		await expectPageContentMounted(page);
		expect(pageErrors).toEqual([]);
	});

	test("API endpoints work without auth on localhost", async ({ request }) => {
		// Protected endpoints should work without auth on localhost
		const resp = await request.get("/api/bootstrap");
		expect(resp.ok()).toBeTruthy();
	});

	test("auth disabled banner not shown on localhost", async ({ page }) => {
		await page.goto("/");
		await expectPageContentMounted(page);

		// The auth-disabled banner should not be visible on localhost default config
		const banner = page.locator("#authDisabledBanner");
		await expect(banner).toBeHidden();
	});

	test("setup page is accessible", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/setup");
		await page.waitForLoadState("networkidle");
		await expect
			.poll(() => {
				const pathname = new URL(page.url()).pathname;
				return /^\/setup$/.test(pathname) || /^\/onboarding$/.test(pathname) || /^\/chats\//.test(pathname);
			})
			.toBeTruthy();

		const pathname = new URL(page.url()).pathname;
		if (/^\/chats\//.test(pathname)) {
			await expectPageContentMounted(page);
		} else {
			await expect(
				page.getByRole("heading", {
					name: /Secure your instance|Set up your identity/,
				}),
			).toBeVisible();
		}

		expect(pageErrors).toEqual([]);
	});

	test("security settings page shows auth options", async ({ page }) => {
		await page.goto("/settings/security");
		await expectPageContentMounted(page);

		await expect(page.getByRole("heading", { name: "Security" })).toBeVisible();
	});
});
