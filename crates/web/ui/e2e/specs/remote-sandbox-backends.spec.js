const { expect, test } = require("../base-test");
const { navigateAndWait, watchPageErrors } = require("../helpers");

async function openSandboxTab(page, tabName) {
	await navigateAndWait(page, "/settings/sandboxes");
	const tab = page.getByRole("tab", { name: tabName, exact: true });
	await tab.click();
	await expect(tab).toHaveAttribute("aria-selected", "true");
}

function backendSection(page, heading) {
	return page.locator("div.max-w-form", {
		has: page.getByRole("heading", { name: heading, exact: true }),
	});
}

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
						coder: {
							configured: false,
							url_configured: false,
							url_from_env: false,
							token_configured: false,
							token_from_env: false,
							template_configured: false,
							user: "me",
							workspace_prefix: "moltis",
						},
					}),
				});
			}
			return route.continue();
		});
	});

	test.afterEach(async ({ page }) => {
		await page.unrouteAll({ behavior: "ignoreErrors" }).catch(() => undefined);
	});

	test("remote backends section is visible on sandbox settings page", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/sandboxes");

		await expect(page.getByRole("tab", { name: "Vercel", exact: true })).toBeVisible();
		await expect(page.getByRole("tab", { name: "Daytona", exact: true })).toBeVisible();
		await expect(page.getByRole("tab", { name: "Coder", exact: true })).toBeVisible();
		await page.getByRole("tab", { name: "Vercel", exact: true }).click();
		await expect(page.getByRole("heading", { name: "Vercel Sandbox", exact: true })).toBeVisible();
		await page.getByRole("tab", { name: "Daytona", exact: true }).click();
		await expect(page.getByRole("heading", { name: "Daytona", exact: true })).toBeVisible();
		await page.getByRole("tab", { name: "Coder", exact: true }).click();
		await expect(page.getByRole("heading", { name: "Coder", exact: true })).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("shows not-configured badges when no credentials are set", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await openSandboxTab(page, "Vercel");

		await expect(backendSection(page, "Vercel Sandbox").getByText("not configured")).toBeVisible();
		await page.getByRole("tab", { name: "Daytona", exact: true }).click();
		await expect(backendSection(page, "Daytona").getByText("not configured")).toBeVisible();
		await page.getByRole("tab", { name: "Coder", exact: true }).click();
		await expect(backendSection(page, "Coder").getByText("not configured")).toBeVisible();
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

		await openSandboxTab(page, "Vercel");
		const section = backendSection(page, "Vercel Sandbox");

		// Fill Vercel token
		const tokenInput = section.getByLabel("Vercel token", { exact: true });
		await tokenInput.fill("ver_test_token_12345");
		await section.getByLabel("Project ID", { exact: true }).fill("prj_test_12345");

		// Click save
		const saveBtn = section.getByRole("button", { name: "Save", exact: true });
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
		expect(savedBody.config.project_id).toBe("prj_test_12345");

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

		await openSandboxTab(page, "Daytona");
		const section = backendSection(page, "Daytona");

		// Fill Daytona API key
		const keyInput = section.getByLabel("Daytona API key", { exact: true });
		await keyInput.fill("dyt_test_key_67890");

		// Click save
		const saveBtn = section.getByRole("button", { name: "Save", exact: true });
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
		await openSandboxTab(page, "Daytona");
		const section = backendSection(page, "Daytona");

		// Daytona save button should be disabled without API key
		const daytonaSave = section.getByRole("button", { name: "Save", exact: true });
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

		await openSandboxTab(page, "Vercel");
		const section = backendSection(page, "Vercel Sandbox");

		const tokenInput = section.getByLabel("Vercel token", { exact: true });
		await tokenInput.fill("ver_will_fail");
		await section.getByLabel("Project ID", { exact: true }).fill("prj_will_fail");

		const saveBtn = section.getByRole("button", { name: "Save", exact: true });
		await saveBtn.click();

		// Verify error message is shown
		await expect(page.getByRole("alert")).toHaveText("Permission denied");

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

		await openSandboxTab(page, "Vercel");
		const section = backendSection(page, "Vercel Sandbox");

		// Fill all Vercel fields
		await section.getByLabel("Vercel token", { exact: true }).fill("ver_abc");
		await section.getByLabel("Project ID", { exact: true }).fill("prj_123");
		await section.getByLabel("Team ID", { exact: true }).fill("team_456");

		await section.getByRole("button", { name: "Save", exact: true }).click();
		await expect(page.getByText("vercel configuration saved")).toBeVisible({ timeout: 5000 });

		expect(savedBody.config.token).toBe("ver_abc");
		expect(savedBody.config.project_id).toBe("prj_123");
		expect(savedBody.config.team_id).toBe("team_456");

		expect(pageErrors).toEqual([]);
	});

	test("saving Coder sends core and optional configuration", async ({ page }) => {
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
						config: {
							vercel: { configured: false, runtime: "node24", timeout_ms: 300000, vcpus: 2 },
							daytona: { configured: false, api_url: "https://app.daytona.io/api" },
							coder: {
								configured: true,
								url_configured: true,
								url_from_env: false,
								url: "https://coder.example.com",
								token_configured: true,
								token_from_env: false,
								template_configured: true,
								organization: "engineering",
								user: "me",
								template_name: "devbox",
								workspace_prefix: "moltis-ci",
								ttl_ms: 600000,
								size: "large",
							},
						},
					}),
				});
			}
			return route.continue();
		});

		await openSandboxTab(page, "Coder");
		const section = backendSection(page, "Coder");
		await section.getByLabel("Coder URL", { exact: true }).fill("https://coder.example.com");
		await section.getByLabel("Coder session token", { exact: true }).fill("coder_test_token");
		await section.getByLabel("Template name", { exact: true }).fill("devbox");
		await section.getByLabel("Organization", { exact: true }).fill("engineering");
		await section.getByLabel("Workspace prefix", { exact: true }).fill("moltis-ci");
		await section.getByLabel("TTL (milliseconds)", { exact: true }).fill("600000");
		await section.getByLabel("Size or preset", { exact: true }).fill("large");
		await section.getByRole("button", { name: "Save", exact: true }).click();

		await expect(page.getByText("coder configuration saved. Restart Moltis to apply.", { exact: true })).toBeVisible();
		expect(savedBody).toEqual({
			backend: "coder",
			config: {
				organization: "engineering",
				user: "me",
				template_id: null,
				template_name: "devbox",
				workspace_prefix: "moltis-ci",
				ttl_ms: 600000,
				size: "large",
				url: "https://coder.example.com",
				token: "coder_test_token",
			},
		});
		expect(pageErrors).toEqual([]);
	});

	test("Coder marks URL and token managed by environment", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.route("**/api/sandbox/remote-backends", (route, request) => {
			if (request.method() !== "GET") return route.continue();
			return route.fulfill({
				status: 200,
				contentType: "application/json",
				body: JSON.stringify({
					vercel: { configured: false, runtime: "node24", timeout_ms: 300000, vcpus: 2 },
					daytona: { configured: false, api_url: "https://app.daytona.io/api" },
					coder: {
						configured: true,
						url_configured: true,
						url_from_env: true,
						url: "https://coder.env.example.com",
						token_configured: true,
						token_from_env: true,
						template_configured: true,
						user: "me",
						template_name: "devbox",
						workspace_prefix: "moltis",
					},
				}),
			});
		});

		await openSandboxTab(page, "Coder");
		const section = backendSection(page, "Coder");
		await expect(section.getByLabel("Coder URL", { exact: true })).toBeDisabled();
		await expect(section.getByLabel("Coder URL", { exact: true })).toHaveValue("https://coder.env.example.com");
		await expect(section.getByLabel("Coder session token", { exact: true })).toBeDisabled();
		await expect(section.getByText("Managed by CODER_URL.", { exact: false })).toBeVisible();
		await expect(section.getByText("Managed by CODER_SESSION_TOKEN.", { exact: false })).toBeVisible();
		await expect(section.getByRole("button", { name: "Save", exact: true })).toBeEnabled();
		expect(pageErrors).toEqual([]);
	});

	test("Coder badge and save readiness require a template", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.route("**/api/sandbox/remote-backends", (route, request) => {
			if (request.method() !== "GET") return route.continue();
			return route.fulfill({
				status: 200,
				contentType: "application/json",
				body: JSON.stringify({
					vercel: { configured: false, runtime: "node24", timeout_ms: 300000, vcpus: 2 },
					daytona: { configured: false, api_url: "https://app.daytona.io/api" },
					coder: {
						configured: false,
						url_configured: true,
						url_from_env: false,
						url: "https://coder.example.com",
						token_configured: true,
						token_from_env: false,
						template_configured: false,
						user: "me",
						workspace_prefix: "moltis",
					},
				}),
			});
		});

		await openSandboxTab(page, "Coder");
		const section = backendSection(page, "Coder");
		await expect(section.getByText("not configured", { exact: true })).toBeVisible();
		await expect(section.getByRole("button", { name: "Save", exact: true })).toBeDisabled();
		await section.getByLabel("Template name", { exact: true }).fill("devbox");
		await expect(section.getByRole("button", { name: "Save", exact: true })).toBeEnabled();
		expect(pageErrors).toEqual([]);
	});

	test("Coder explicit config stays editable when stale environment aliases do not win", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.route("**/api/sandbox/remote-backends", (route, request) => {
			if (request.method() !== "GET") return route.continue();
			return route.fulfill({
				status: 200,
				contentType: "application/json",
				body: JSON.stringify({
					vercel: { configured: false, runtime: "node24", timeout_ms: 300000, vcpus: 2 },
					daytona: { configured: false, api_url: "https://app.daytona.io/api" },
					coder: {
						configured: true,
						url_configured: true,
						url_from_env: false,
						url: "https://configured.example.com",
						token_configured: true,
						token_from_env: false,
						template_configured: true,
						template_name: "devbox",
						user: "me",
						workspace_prefix: "moltis",
					},
				}),
			});
		});

		await openSandboxTab(page, "Coder");
		const section = backendSection(page, "Coder");
		await expect(section.getByLabel("Coder URL", { exact: true })).toBeEnabled();
		await expect(section.getByLabel("Coder URL", { exact: true })).toHaveValue("https://configured.example.com");
		await expect(section.getByLabel("Coder session token", { exact: true })).toBeEnabled();
		await expect(section.getByText("configured", { exact: true })).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("Coder rejects whitespace tokens and negative TTL in the form", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await openSandboxTab(page, "Coder");
		const section = backendSection(page, "Coder");
		const save = section.getByRole("button", { name: "Save", exact: true });
		await section.getByLabel("Coder URL", { exact: true }).fill("https://coder.example.com");
		await section.getByLabel("Coder session token", { exact: true }).fill("   ");
		await section.getByLabel("Template ID", { exact: true }).fill("template-id");
		await expect(save).toBeDisabled();

		await section.getByLabel("Coder session token", { exact: true }).fill("coder-token");
		await section.getByLabel("TTL (milliseconds)", { exact: true }).fill("-1");
		await expect(section.getByText("TTL must be a non-negative whole number.", { exact: true })).toBeVisible();
		await expect(save).toBeDisabled();

		await section.getByLabel("TTL (milliseconds)", { exact: true }).fill("0");
		await expect(section.getByText("Zero disables automatic workspace shutdown.", { exact: true })).toBeVisible();
		await expect(save).toBeEnabled();
		expect(pageErrors).toEqual([]);
	});

	test("Coder displays insecure URL validation errors without JavaScript errors", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.route("**/api/sandbox/remote-backends", (route, request) => {
			if (request.method() !== "PUT") return route.continue();
			return route.fulfill({
				status: 400,
				contentType: "application/json",
				body: JSON.stringify({
					code: "remote_backend_invalid_config",
					error: "coder_url must use HTTPS; HTTP is allowed only for localhost or a literal loopback address",
				}),
			});
		});

		await openSandboxTab(page, "Coder");
		const section = backendSection(page, "Coder");
		await section.getByLabel("Coder URL", { exact: true }).fill("http://coder.example.com");
		await section.getByLabel("Coder session token", { exact: true }).fill("coder_test_token");
		await section.getByLabel("Template ID", { exact: true }).fill("template-id");
		await section.getByRole("button", { name: "Save", exact: true }).click();

		await expect(section.getByRole("alert")).toContainText("HTTPS");
		expect(pageErrors).toEqual([]);
	});
});
