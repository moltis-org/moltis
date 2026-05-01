const { expect, test } = require("../base-test");
const { navigateAndWait, watchPageErrors } = require("../helpers");

test.describe("Remote sandbox backend configuration", () => {
	test.beforeEach(async ({ page }) => {
		// Mock the GET endpoint to simulate no backends configured initially.
		await page.route("**/api/sandbox/remote-backends", (route, request) => {
			if (request.method() === "GET") {
				return route.fulfill({
					status: 200,
					contentType: "application/json",
					body: JSON.stringify({
						vercel: { configured: false, runtime: "node24", timeout_ms: 300000, vcpus: 2 },
						daytona: { configured: false, api_url: "https://app.daytona.io/api" },
					}),
				});
			}
			return route.continue();
		});
	});

	test.afterEach(async ({ page }) => {
		await page.unrouteAll({ behavior: "ignoreErrors" }).catch(() => {});
	});

	test("remote backends section is visible on sandbox settings page", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/sandboxes");

		await expect(page.getByText("Remote sandbox backends")).toBeVisible();
		await expect(page.getByText("Vercel Sandbox")).toBeVisible();
		await expect(page.getByText("Daytona")).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("shows not-configured badges when no credentials are set", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/sandboxes");

		const badges = page.getByText("not configured");
		await expect(badges.first()).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("saving Vercel token shows success message and configured badge", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		let savedBody = null;

		await page.route("**/api/sandbox/remote-backends", (route, request) => {
			if (request.method() === "PUT") {
				savedBody = request.postDataJSON();
				return route.fulfill({
					status: 200,
					contentType: "application/json",
					body: JSON.stringify({
						ok: true,
						restart_required: true,
						config_path: "/test/moltis.toml",
						config: {
							vercel: { configured: true, runtime: "node24", timeout_ms: 300000, vcpus: 2 },
							daytona: { configured: false, api_url: "https://app.daytona.io/api" },
						},
					}),
				});
			}
			if (request.method() === "GET") {
				return route.fulfill({
					status: 200,
					contentType: "application/json",
					body: JSON.stringify({
						vercel: { configured: false, runtime: "node24", timeout_ms: 300000, vcpus: 2 },
						daytona: { configured: false, api_url: "https://app.daytona.io/api" },
					}),
				});
			}
			return route.continue();
		});

		await navigateAndWait(page, "/settings/sandboxes");

		// Fill Vercel token
		const tokenInput = page.locator('input[placeholder*="Vercel token"]');
		await tokenInput.fill("ver_test_token_12345");

		// Click save
		const saveBtn = page.getByRole("button", { name: "Save Vercel", exact: true });
		await expect(saveBtn).toBeEnabled();
		await saveBtn.click();

		// Verify success message
		await expect(page.getByText("vercel configuration saved")).toBeVisible({ timeout: 5000 });

		// Verify configured badge appears
		await expect(page.getByText("configured").first()).toBeVisible();

		// Verify the request was sent correctly
		expect(savedBody).not.toBeNull();
		expect(savedBody.backend).toBe("vercel");
		expect(savedBody.config.token).toBe("ver_test_token_12345");

		expect(pageErrors).toEqual([]);
	});

	test("saving Daytona API key shows success message", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		let savedBody = null;

		await page.route("**/api/sandbox/remote-backends", (route, request) => {
			if (request.method() === "PUT") {
				savedBody = request.postDataJSON();
				return route.fulfill({
					status: 200,
					contentType: "application/json",
					body: JSON.stringify({
						ok: true,
						restart_required: true,
						config_path: "/test/moltis.toml",
						config: {
							vercel: { configured: false, runtime: "node24", timeout_ms: 300000, vcpus: 2 },
							daytona: { configured: true, api_url: "https://app.daytona.io/api" },
						},
					}),
				});
			}
			if (request.method() === "GET") {
				return route.fulfill({
					status: 200,
					contentType: "application/json",
					body: JSON.stringify({
						vercel: { configured: false, runtime: "node24", timeout_ms: 300000, vcpus: 2 },
						daytona: { configured: false, api_url: "https://app.daytona.io/api" },
					}),
				});
			}
			return route.continue();
		});

		await navigateAndWait(page, "/settings/sandboxes");

		// Fill Daytona API key
		const keyInput = page.locator('input[placeholder*="Daytona API key"]');
		await keyInput.fill("dyt_test_key_67890");

		// Click save
		const saveBtn = page.getByRole("button", { name: "Save Daytona", exact: true });
		await expect(saveBtn).toBeEnabled();
		await saveBtn.click();

		// Verify success message
		await expect(page.getByText("daytona configuration saved")).toBeVisible({ timeout: 5000 });

		// Verify request
		expect(savedBody).not.toBeNull();
		expect(savedBody.backend).toBe("daytona");
		expect(savedBody.config.api_key).toBe("dyt_test_key_67890");

		expect(pageErrors).toEqual([]);
	});

	test("save button is disabled when token field is empty", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/sandboxes");

		// Daytona save button should be disabled without API key
		const daytonaSave = page.getByRole("button", { name: "Save Daytona", exact: true });
		await expect(daytonaSave).toBeDisabled();

		expect(pageErrors).toEqual([]);
	});

	test("API error displays error message", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await page.route("**/api/sandbox/remote-backends", (route, request) => {
			if (request.method() === "PUT") {
				return route.fulfill({
					status: 500,
					contentType: "application/json",
					body: JSON.stringify({ code: "save_failed", error: "Permission denied" }),
				});
			}
			if (request.method() === "GET") {
				return route.fulfill({
					status: 200,
					contentType: "application/json",
					body: JSON.stringify({
						vercel: { configured: false, runtime: "node24", timeout_ms: 300000, vcpus: 2 },
						daytona: { configured: false, api_url: "https://app.daytona.io/api" },
					}),
				});
			}
			return route.continue();
		});

		await navigateAndWait(page, "/settings/sandboxes");

		const tokenInput = page.locator('input[placeholder*="Vercel token"]');
		await tokenInput.fill("ver_will_fail");

		const saveBtn = page.getByRole("button", { name: "Save Vercel", exact: true });
		await saveBtn.click();

		// Verify error message is shown
		await expect(page.locator(".alert-error-text")).toBeVisible({ timeout: 5000 });

		expect(pageErrors).toEqual([]);
	});

	test("Vercel project ID and team ID are sent with save", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		let savedBody = null;

		await page.route("**/api/sandbox/remote-backends", (route, request) => {
			if (request.method() === "PUT") {
				savedBody = request.postDataJSON();
				return route.fulfill({
					status: 200,
					contentType: "application/json",
					body: JSON.stringify({
						ok: true,
						restart_required: true,
						config_path: "/test/moltis.toml",
						config: {
							vercel: {
								configured: true,
								project_id: "prj_123",
								team_id: "team_456",
								runtime: "node24",
								timeout_ms: 300000,
								vcpus: 2,
							},
							daytona: { configured: false, api_url: "https://app.daytona.io/api" },
						},
					}),
				});
			}
			if (request.method() === "GET") {
				return route.fulfill({
					status: 200,
					contentType: "application/json",
					body: JSON.stringify({
						vercel: { configured: false, runtime: "node24", timeout_ms: 300000, vcpus: 2 },
						daytona: { configured: false, api_url: "https://app.daytona.io/api" },
					}),
				});
			}
			return route.continue();
		});

		await navigateAndWait(page, "/settings/sandboxes");

		// Fill all Vercel fields
		await page.locator('input[placeholder*="Vercel token"]').fill("ver_abc");
		await page.locator('input[placeholder*="Project ID"]').fill("prj_123");
		await page.locator('input[placeholder*="Team ID"]').fill("team_456");

		await page.getByRole("button", { name: "Save Vercel", exact: true }).click();
		await expect(page.getByText("vercel configuration saved")).toBeVisible({ timeout: 5000 });

		expect(savedBody.config.token).toBe("ver_abc");
		expect(savedBody.config.project_id).toBe("prj_123");
		expect(savedBody.config.team_id).toBe("team_456");

		expect(pageErrors).toEqual([]);
	});
});
