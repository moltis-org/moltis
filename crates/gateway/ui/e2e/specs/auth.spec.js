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

	test("page title uses configured identity name", async ({ page }) => {
		await page.goto("/");
		await expectPageContentMounted(page);

		// The E2E server seeds IDENTITY.md with name: e2e-bot
		await expect(page).toHaveTitle(/e2e-bot/);
	});
});

/**
 * Login page tests. The login page is a standalone HTML page (login.html)
 * served at /login, separate from the main SPA. It fetches /api/auth/status
 * to determine which auth methods to show.
 *
 * We use addInitScript to inject fetch mocks directly into the page's JS
 * context before any module scripts run. This is more reliable than
 * page.route() for standalone pages with module scripts.
 */
test.describe("Login page", () => {
	/**
	 * Mock auth status via init script. Overrides fetch() in the page
	 * context BEFORE any module scripts run, ensuring the mock intercepts
	 * when login-app.js fetches /api/auth/status.
	 */
	function mockAuthStatus(page, overrides = {}) {
		const defaults = {
			authenticated: false,
			setup_required: false,
			auth_disabled: false,
			localhost_only: false,
			has_password: true,
			has_passkeys: false,
		};
		const status = { ...defaults, ...overrides };
		return page.addInitScript((mockStatus) => {
			const origFetch = window.fetch;
			window.fetch = function (...args) {
				var url = typeof args[0] === "string" ? args[0] : args[0].url;
				if (url.endsWith("/api/auth/status")) {
					return Promise.resolve(
						new Response(JSON.stringify(mockStatus), {
							status: 200,
							headers: { "Content-Type": "application/json" },
						}),
					);
				}
				return origFetch.apply(this, args);
			};
		}, status);
	}

	test("login page is a standalone page without app chrome", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await mockAuthStatus(page);

		await page.goto("/login");
		await expect(page.locator(".auth-card")).toBeVisible();

		// Login page is standalone — no header, nav, or sessions panel in the DOM
		expect(await page.locator("header").count()).toBe(0);
		expect(await page.locator("#navPanel").count()).toBe(0);
		expect(await page.locator("#sessionsPanel").count()).toBe(0);

		expect(pageErrors).toEqual([]);
	});

	test("login page renders password form", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await mockAuthStatus(page, { has_password: true, has_passkeys: false });

		await page.goto("/login");

		await expect(page.getByPlaceholder("Enter password")).toBeVisible();
		await expect(page.getByRole("button", { name: "Sign in", exact: true })).toBeVisible();
		await expect(page.getByRole("button", { name: "Sign in with passkey" })).not.toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("login page title uses identity name from gon data", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await mockAuthStatus(page);

		await page.goto("/login");
		await expect(page.locator(".auth-card")).toBeVisible();

		const title = await page.locator(".auth-title").textContent();
		expect(title).toContain("e2e-bot");

		expect(pageErrors).toEqual([]);
	});

	test("login page shows subtitle", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await mockAuthStatus(page);

		await page.goto("/login");
		await expect(page.locator(".auth-subtitle")).toContainText("Sign in to continue");

		expect(pageErrors).toEqual([]);
	});

	test("login page shows both methods when password and passkeys are set", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await mockAuthStatus(page, { has_password: true, has_passkeys: true });

		await page.goto("/login");

		await expect(page.getByPlaceholder("Enter password")).toBeVisible();
		await expect(page.getByRole("button", { name: "Sign in", exact: true })).toBeVisible();
		await expect(page.getByRole("button", { name: "Sign in with passkey" })).toBeVisible();
		await expect(page.locator(".auth-divider")).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("login page shows only passkey when no password is set", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await mockAuthStatus(page, { has_password: false, has_passkeys: true });

		await page.goto("/login");

		await expect(page.getByRole("button", { name: "Sign in with passkey" })).toBeVisible();
		await expect(page.getByPlaceholder("Enter password")).not.toBeVisible();
		await expect(page.locator(".auth-divider")).not.toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("login page shows error on wrong password", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		// Mock both auth status and login endpoints via init script
		await page.addInitScript(() => {
			var origFetch = window.fetch;
			window.fetch = function (...args) {
				var url = typeof args[0] === "string" ? args[0] : args[0].url;
				if (url.endsWith("/api/auth/status")) {
					return Promise.resolve(
						new Response(
							JSON.stringify({
								authenticated: false,
								setup_required: false,
								auth_disabled: false,
								localhost_only: false,
								has_password: true,
								has_passkeys: false,
							}),
							{ status: 200, headers: { "Content-Type": "application/json" } },
						),
					);
				}
				if (url.endsWith("/api/auth/login")) {
					return Promise.resolve(
						new Response(JSON.stringify({ error: "invalid password" }), {
							status: 401,
							headers: { "Content-Type": "application/json" },
						}),
					);
				}
				return origFetch.apply(this, args);
			};
		});

		await page.goto("/login");

		await page.getByPlaceholder("Enter password").fill("wrong-password");
		await page.getByRole("button", { name: "Sign in", exact: true }).click();

		await expect(page.locator(".auth-error")).toContainText("Invalid password");
		expect(pageErrors).toEqual([]);
	});

	test("login page redirects to / when already authenticated", async ({ page }) => {
		// On the test server, /api/auth/status returns authenticated: true
		// (localhost bypass). The login page should detect this and redirect.
		const pageErrors = watchPageErrors(page);
		await page.goto("/login");
		await expect(page).toHaveURL(/\/chats\//);
		expect(pageErrors).toEqual([]);
	});

	test("auth status API provides required fields for login page", async ({ request }) => {
		const resp = await request.get("/api/auth/status");
		expect(resp.ok()).toBeTruthy();
		const data = await resp.json();
		expect(data).toHaveProperty("authenticated");
		expect(data).toHaveProperty("has_password");
		expect(data).toHaveProperty("has_passkeys");
	});
});
