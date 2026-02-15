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
		await navigateAndWait(page, "/settings/sandboxes");

		// Wait for the containers fetch to complete
		await page.waitForResponse((r) => r.url().includes("/api/sandbox/containers"));

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
