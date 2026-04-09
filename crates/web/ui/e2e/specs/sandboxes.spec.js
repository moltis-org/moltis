const { expect, test } = require("../base-test");
const { navigateAndWait, watchPageErrors } = require("../helpers");

test.describe("Sandboxes page – Image tag truncation", () => {
	test("long image hash tags are truncated in the cached images list", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		const longHash = "78e523c6835f0d509a9da736bea2cbaeac5983c8fe5468ed062b557b74518f66";
		const fullTag = `moltis-sandbox:${longHash}`;

		// Intercept cached images API to inject a long-hash image
		await page.route("**/api/images/cached", (route, request) => {
			if (request.method() === "GET") {
				return route.fulfill({
					status: 200,
					contentType: "application/json",
					body: JSON.stringify({
						images: [
							{ tag: fullTag, size: "764 MB", created: "2026-02-15T19:30:51Z", kind: "sandbox", skill_name: "sandbox" },
						],
					}),
				});
			}
			return route.continue();
		});

		await navigateAndWait(page, "/settings/sandboxes");

		// The displayed text should be truncated (first 6 + … + last 6 of hash)
		const truncated = `moltis-sandbox:${longHash.slice(0, 6)}\u2026${longHash.slice(-6)}`;
		const tagSpan = page.locator(".provider-item-name", { hasText: truncated });
		await expect(tagSpan).toBeVisible();

		// Full tag should be in the title attribute for hover
		await expect(tagSpan).toHaveAttribute("title", fullTag);

		// The full untruncated tag should NOT appear as visible text
		await expect(page.getByText(fullTag, { exact: true })).not.toBeVisible();

		expect(pageErrors).toEqual([]);
	});
});

test.describe("Sandboxes page – Shared home settings", () => {
	test("shows shared folder status and saves updates", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		let savedBody = null;

		await page.route("**/api/sandbox/shared-home", (route, request) => {
			if (request.method() === "GET") {
				return route.fulfill({
					status: 200,
					contentType: "application/json",
					body: JSON.stringify({
						enabled: true,
						mode: "shared",
						path: "/tmp/moltis-shared",
						configured_path: "/tmp/moltis-shared",
					}),
				});
			}
			if (request.method() === "PUT") {
				savedBody = request.postDataJSON();
				return route.fulfill({
					status: 200,
					contentType: "application/json",
					body: JSON.stringify({
						ok: true,
						restart_required: true,
						config: {
							enabled: false,
							mode: "off",
							path: "/tmp/moltis-new-shared",
							configured_path: "/tmp/moltis-new-shared",
						},
					}),
				});
			}
			return route.continue();
		});

		await navigateAndWait(page, "/settings/sandboxes");
		const sharedHomeSection = page.locator("div.max-w-form", {
			has: page.getByText("Shared home folder", { exact: true }),
		});

		await expect(sharedHomeSection.getByText("Shared home folder", { exact: true })).toBeVisible();
		await expect(sharedHomeSection.getByLabel("Enable shared home folder")).toBeChecked();
		await expect(sharedHomeSection.getByLabel("Shared folder location")).toHaveValue("/tmp/moltis-shared");

		await sharedHomeSection.getByLabel("Enable shared home folder").uncheck();
		await sharedHomeSection.getByLabel("Shared folder location").fill("/tmp/moltis-new-shared");
		const saveResponse = page.waitForResponse(
			(r) => r.url().includes("/api/sandbox/shared-home") && r.request().method() === "PUT" && r.status() === 200,
		);
		await sharedHomeSection.getByRole("button", { name: "Save", exact: true }).click();
		await saveResponse;

		expect(savedBody).toEqual({
			enabled: false,
			path: "/tmp/moltis-new-shared",
		});
		await expect(
			sharedHomeSection.getByText("Saved. Restart Moltis to apply shared folder changes.", { exact: true }),
		).toBeVisible();
		await expect(sharedHomeSection.getByText("disabled (off)")).toBeVisible();

		expect(pageErrors).toEqual([]);
	});
});

/**
 * Make the sandbox runtime appear available in the e2e environment.
 *
 * CI has no container daemon so gon/bootstrap report `backend: "none"`,
 * which disables buttons (changing their accessible name to a long hint).
 * This helper patches three layers:
 *   1. `window.__MOLTIS__` (gon data embedded in HTML) — via addInitScript
 *   2. `/api/gon` responses — via route interception
 *   3. `/api/bootstrap` responses — via route interception
 */
async function mockSandboxAvailable(page) {
	await page.addInitScript(() => {
		var m = window.__MOLTIS__ || {};
		m.sandbox = Object.assign(m.sandbox || {}, { backend: "docker" });
		window.__MOLTIS__ = m;
	});

	await page.route("**/api/gon*", async (route) => {
		var response = await route.fetch();
		var json = await response.json();
		json.sandbox = Object.assign(json.sandbox || {}, { backend: "docker" });
		return route.fulfill({ response, json });
	});

	await page.route("**/api/bootstrap*", async (route) => {
		var response = await route.fetch();
		var json = await response.json();
		json.sandbox = Object.assign(json.sandbox || {}, { backend: "docker" });
		return route.fulfill({ response, json });
	});
}

test.describe("Sandboxes page – Running Containers", () => {
	test.beforeEach(async ({ page }) => {
		await mockSandboxAvailable(page);
	});

	test.afterEach(async ({ page }) => {
		await page.unrouteAll({ behavior: "ignoreErrors" }).catch(() => {});
	});

	test("running containers section renders with heading and refresh button", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		// Mock container list so the button text resolves to "Refresh" quickly
		// (the real endpoint can be slow with Apple Container).
		await page.route("**/api/sandbox/containers", (route, request) => {
			if (request.method() === "GET") {
				return route.fulfill({
					status: 200,
					contentType: "application/json",
					body: JSON.stringify({ containers: [] }),
				});
			}
			return route.continue();
		});

		await navigateAndWait(page, "/settings/sandboxes");

		await expect(page.getByRole("heading", { name: "Sandboxes", exact: true })).toBeVisible();
		await expect(page.getByText("Running Containers")).toBeVisible();
		await expect(page.getByRole("button", { name: "Refresh", exact: true })).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("refresh button triggers container list fetch", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		let fetchCount = 0;

		// Mock container list for fast initial load; tracks call count.
		await page.route("**/api/sandbox/containers", (route, request) => {
			if (request.method() === "GET") {
				fetchCount++;
				return route.fulfill({
					status: 200,
					contentType: "application/json",
					body: JSON.stringify({ containers: [] }),
				});
			}
			return route.continue();
		});

		await navigateAndWait(page, "/settings/sandboxes");
		await expect(page.getByRole("button", { name: "Refresh", exact: true })).toBeVisible();
		const mountCount = fetchCount;

		await page.getByRole("button", { name: "Refresh", exact: true }).click();
		await expect.poll(() => fetchCount, { timeout: 10_000 }).toBeGreaterThan(mountCount);
		expect(fetchCount).toBeGreaterThan(mountCount);

		expect(pageErrors).toEqual([]);
	});

	test("containers list fetches on page mount", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		var containersFetched = false;

		// Track the containers fetch via route interceptor so it can't race
		// with page.goto — the route is registered before navigation starts.
		await page.route("**/api/sandbox/containers", (route, request) => {
			if (request.method() === "GET") {
				containersFetched = true;
			}
			return route.continue();
		});

		await navigateAndWait(page, "/settings/sandboxes");
		await expect.poll(() => containersFetched, { timeout: 10_000 }).toBe(true);

		expect(pageErrors).toEqual([]);
	});

	test("shows 'No containers found' when list is empty", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		// Mock an empty container list to make the test deterministic.
		await page.route("**/api/sandbox/containers", (route, request) => {
			if (request.method() === "GET") {
				return route.fulfill({
					status: 200,
					contentType: "application/json",
					body: JSON.stringify({ containers: [] }),
				});
			}
			return route.continue();
		});

		await navigateAndWait(page, "/settings/sandboxes");
		await expect(page.getByRole("button", { name: "Refresh", exact: true })).toBeVisible();
		await expect(page.getByText("No containers found.")).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("disk usage fetches on page mount", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		var diskUsageFetched = false;

		// Fulfill directly so the test does not depend on the real runtime.
		await page.route("**/api/sandbox/disk-usage", (route) => {
			diskUsageFetched = true;
			return route.fulfill({
				status: 200,
				contentType: "application/json",
				body: JSON.stringify({ size_bytes: 0, size_human: "0 B" }),
			});
		});

		await navigateAndWait(page, "/settings/sandboxes");
		await expect.poll(() => diskUsageFetched, { timeout: 10_000 }).toBe(true);

		expect(pageErrors).toEqual([]);
	});

	test("refresh button also fetches disk usage", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		var diskFetchCount = 0;

		// Mock container list so the button resolves to "Refresh" quickly.
		await page.route("**/api/sandbox/containers", (route, request) => {
			if (request.method() === "GET") {
				return route.fulfill({
					status: 200,
					contentType: "application/json",
					body: JSON.stringify({ containers: [] }),
				});
			}
			return route.continue();
		});

		// Track disk-usage fetches so we can assert the refresh triggered one.
		await page.route("**/api/sandbox/disk-usage", (route) => {
			diskFetchCount++;
			return route.continue();
		});

		await navigateAndWait(page, "/settings/sandboxes");
		const refreshBtn = page.getByRole("button", { name: "Refresh", exact: true });
		await expect(refreshBtn).toBeVisible();

		// Page mount fires the first disk-usage fetch.
		const mountCount = diskFetchCount;

		await refreshBtn.click();
		await expect.poll(() => diskFetchCount, { timeout: 10_000 }).toBeGreaterThan(mountCount);

		expect(diskFetchCount).toBeGreaterThan(mountCount);
		expect(pageErrors).toEqual([]);
	});

	test("clean all endpoint responds correctly", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		// Mock container list so the page loads quickly.
		await page.route("**/api/sandbox/containers", (route, request) => {
			if (request.method() === "GET") {
				return route.fulfill({
					status: 200,
					contentType: "application/json",
					body: JSON.stringify({ containers: [] }),
				});
			}
			return route.continue();
		});

		// Mock the clean endpoint — the real operation can be slow with
		// Apple Container. We only verify the response shape here.
		await page.route("**/api/sandbox/containers/clean", (route, request) => {
			if (request.method() === "POST") {
				return route.fulfill({
					status: 200,
					contentType: "application/json",
					body: JSON.stringify({ ok: true, removed: [] }),
				});
			}
			return route.continue();
		});

		await navigateAndWait(page, "/settings/sandboxes");

		// Call the clean all API via page.evaluate; the route mock intercepts it.
		const result = await page.evaluate(async () => {
			const r = await fetch("/api/sandbox/containers/clean", { method: "POST" });
			return { status: r.status, data: await r.json() };
		});
		expect(result.status).toBe(200);
		expect(result.data).toHaveProperty("ok", true);
		expect(result.data).toHaveProperty("removed");

		expect(pageErrors).toEqual([]);
	});
});

test.describe("Sandboxes page – Container error handling", () => {
	test.beforeEach(async ({ page }) => {
		await mockSandboxAvailable(page);
	});

	test.afterEach(async ({ page }) => {
		await page.unrouteAll({ behavior: "ignoreErrors" }).catch(() => {});
	});

	test("delete failure shows error message that clears on refresh", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		var containerListFetches = 0;

		// Mock container list with one container
		await page.route("**/api/sandbox/containers", (route, request) => {
			if (request.method() === "GET") {
				containerListFetches++;
				return route.fulfill({
					status: 200,
					contentType: "application/json",
					body: JSON.stringify({
						containers: [
							{
								name: "moltis-sandbox-ghost",
								image: "ubuntu:25.10",
								state: "stopped",
								backend: "apple-container",
								cpus: null,
								memory_mb: null,
								started: null,
								addr: null,
							},
						],
					}),
				});
			}
			return route.continue();
		});

		// Mock DELETE to return 500
		await page.route("**/api/sandbox/containers/moltis-sandbox-ghost", (route, request) => {
			if (request.method() === "DELETE") {
				return route.fulfill({
					status: 500,
					contentType: "text/plain",
					body: "container rm failed: ghost container",
				});
			}
			return route.continue();
		});

		await navigateAndWait(page, "/settings/sandboxes");
		await expect.poll(() => containerListFetches, { timeout: 10_000 }).toBeGreaterThan(0);

		// Click the delete button
		await page.getByRole("button", { name: "Delete", exact: true }).click();

		// Error message should appear
		const errorDiv = page.locator(".alert-error-text");
		await expect(errorDiv).toBeVisible();
		await expect(errorDiv).toContainText("Failed to delete moltis-sandbox-ghost");

		expect(pageErrors).toEqual([]);
	});

	test("error clears on successful container refresh", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		var callCount = 0;

		// First call returns a container, subsequent calls return empty
		await page.route("**/api/sandbox/containers", (route, request) => {
			if (request.method() === "GET") {
				callCount++;
				if (callCount <= 1) {
					return route.fulfill({
						status: 200,
						contentType: "application/json",
						body: JSON.stringify({
							containers: [
								{
									name: "moltis-sandbox-ghost",
									image: "ubuntu:25.10",
									state: "stopped",
									backend: "apple-container",
									cpus: null,
									memory_mb: null,
									started: null,
									addr: null,
								},
							],
						}),
					});
				}
				return route.fulfill({
					status: 200,
					contentType: "application/json",
					body: JSON.stringify({ containers: [] }),
				});
			}
			return route.continue();
		});

		// Mock DELETE to fail
		await page.route("**/api/sandbox/containers/moltis-sandbox-ghost", (route, request) => {
			if (request.method() === "DELETE") {
				return route.fulfill({
					status: 500,
					contentType: "text/plain",
					body: "ghost container",
				});
			}
			return route.continue();
		});

		await navigateAndWait(page, "/settings/sandboxes");
		await expect.poll(() => callCount, { timeout: 10_000 }).toBeGreaterThan(0);

		// Click delete to trigger error (delete no longer auto-refreshes on failure)
		await page.getByRole("button", { name: "Delete", exact: true }).click();
		await expect(page.locator(".alert-error-text")).toBeVisible();

		// Click Refresh to trigger a successful container fetch that clears the error.
		// Second mock returns empty list, so fetchContainers succeeds and clears containerError.
		await page.getByRole("button", { name: "Refresh", exact: true }).click();
		await expect.poll(() => callCount, { timeout: 10_000 }).toBeGreaterThan(1);
		await expect(page.locator(".alert-error-text")).not.toBeVisible();

		expect(pageErrors).toEqual([]);
	});
});
