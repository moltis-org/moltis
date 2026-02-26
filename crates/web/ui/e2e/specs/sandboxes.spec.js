const { expect, test } = require("@playwright/test");
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
		await expect(sharedHomeSection.getByText("Saved. Restart Moltis to apply shared folder changes.", { exact: true })).toBeVisible();
		await expect(sharedHomeSection.getByText("disabled (off)")).toBeVisible();

		expect(pageErrors).toEqual([]);
	});
});

test.describe("Sandboxes page – Running Containers", () => {
	test("running containers section renders with heading and refresh button", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/sandboxes");

		await expect(page.getByRole("heading", { name: "Sandboxes", exact: true })).toBeVisible();
		await expect(page.getByText("Running Containers")).toBeVisible();
		await expect(page.getByRole("button", { name: "Refresh", exact: true })).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("refresh button triggers container list fetch", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/sandboxes");

		const fetchPromise = page.waitForResponse((r) => r.url().includes("/api/sandbox/containers") && r.status() === 200);
		await page.getByRole("button", { name: "Refresh", exact: true }).click();
		const response = await fetchPromise;
		const data = await response.json();
		expect(data).toHaveProperty("containers");
		expect(Array.isArray(data.containers)).toBe(true);

		expect(pageErrors).toEqual([]);
	});

	test("containers list fetches on page mount", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		const fetchPromise = page.waitForResponse((r) => r.url().includes("/api/sandbox/containers") && r.status() === 200);
		await page.goto("/settings/sandboxes");
		const response = await fetchPromise;
		const data = await response.json();
		expect(data).toHaveProperty("containers");

		expect(pageErrors).toEqual([]);
	});

	test("shows 'No containers found' when list is empty", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		const containersResponse = page.waitForResponse(
			(r) => r.url().includes("/api/sandbox/containers") && r.request().method() === "GET",
		);
		await navigateAndWait(page, "/settings/sandboxes");
		await containersResponse;

		// If no containers are running, we should see the empty state
		const containerRows = page.locator(".provider-item");
		const noContainersText = page.getByText("No containers found.");
		// Either containers exist or the empty message shows
		const hasContainers = (await containerRows.count()) > 0;
		if (!hasContainers) {
			await expect(noContainersText).toBeVisible();
		}

		expect(pageErrors).toEqual([]);
	});

	test("disk usage fetches on page mount", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		const fetchPromise = page.waitForResponse((r) => r.url().includes("/api/sandbox/disk-usage"));
		await page.goto("/settings/sandboxes");
		const response = await fetchPromise;
		const data = await response.json();
		// Response should have a usage object (or error if no backend)
		expect(data).toBeDefined();

		expect(pageErrors).toEqual([]);
	});

	test("refresh button also fetches disk usage", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/sandboxes");

		const diskPromise = page.waitForResponse((r) => r.url().includes("/api/sandbox/disk-usage"));
		await page.getByRole("button", { name: "Refresh", exact: true }).click();
		await diskPromise;

		expect(pageErrors).toEqual([]);
	});

	test("clean all endpoint responds correctly", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/sandboxes");

		// Call the clean all API directly to verify the endpoint works
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
	test("delete failure shows error message that clears on refresh", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		// Mock container list with one container
		await page.route("**/api/sandbox/containers", (route, request) => {
			if (request.method() === "GET") {
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

		const containerListResponse = page.waitForResponse(
			(r) => r.url().includes("/api/sandbox/containers") && r.request().method() === "GET",
		);
		await navigateAndWait(page, "/settings/sandboxes");
		await containerListResponse;

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

		const containerListResponse = page.waitForResponse(
			(r) => r.url().includes("/api/sandbox/containers") && r.request().method() === "GET",
		);
		await navigateAndWait(page, "/settings/sandboxes");
		await containerListResponse;

		// Click delete to trigger error (delete no longer auto-refreshes on failure)
		await page.getByRole("button", { name: "Delete", exact: true }).click();
		await expect(page.locator(".alert-error-text")).toBeVisible();

		// Click Refresh to trigger a successful container fetch that clears the error.
		// Second mock returns empty list, so fetchContainers succeeds and clears containerError.
		const refreshResponse = page.waitForResponse(
			(r) => r.url().includes("/api/sandbox/containers") && r.request().method() === "GET",
		);
		await page.getByRole("button", { name: "Refresh", exact: true }).click();
		await refreshResponse;
		await expect(page.locator(".alert-error-text")).not.toBeVisible();

		expect(pageErrors).toEqual([]);
	});
});
