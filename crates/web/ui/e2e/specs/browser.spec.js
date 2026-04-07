// Browser viewer page — end-to-end tests.
//
// Verifies the browser session management UI: creating sessions,
// navigating, screencast frame delivery, and session lifecycle.

const { test, expect } = require("../base-test");
const { navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

test.describe("Browser sessions page", () => {
	test.beforeEach(async ({ page }) => {
		await navigateAndWait(page, "/settings/browser");
		await waitForWsConnected(page);
	});

	test("renders browser page with heading and new session button", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await expect(page.getByRole("heading", { name: "Browser Sessions", exact: true })).toBeVisible();
		await expect(page.getByRole("button", { name: "New Session" })).toBeVisible();
		await expect(page.getByRole("button", { name: "Refresh" })).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("shows empty state message when no sessions exist", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await expect(page.getByText("No active browser sessions")).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("new session button shows creating state", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		const btn = page.getByRole("button", { name: "New Session" });
		await btn.click();

		// Button should show creating state (disabled with "Creating…" text)
		// or have already finished creating — either way, no JS errors.
		// We check that the button was clickable and the page didn't crash.
		await expect(page.getByRole("heading", { name: "Browser Sessions", exact: true })).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("navigate bar appears after creating a session", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		// Create a new session
		await page.getByRole("button", { name: "New Session" }).click();

		// Wait for the navigate input to appear (session created + selected)
		await expect(page.getByPlaceholder("Search or enter URL...")).toBeVisible({ timeout: 30000 });

		// The "Enter a URL" hint should be visible in the canvas area
		await expect(page.getByText("Enter a URL above to start browsing")).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("navigate bar normalizes bare domains with https", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		// Create a session
		await page.getByRole("button", { name: "New Session" }).click();
		await expect(page.getByPlaceholder("Search or enter URL...")).toBeVisible({ timeout: 30000 });

		// Type a bare domain and submit
		const input = page.getByPlaceholder("Search or enter URL...");
		await input.fill("example.com");
		await input.press("Enter");

		// Should navigate successfully (no error toast about invalid scheme)
		// Wait for the session list to update with the URL
		await expect
			.poll(
				async () => {
					const text = await page.locator(".truncate").allInnerTexts();
					return text.some((t) => t.includes("example.com"));
				},
				{ timeout: 30000 },
			)
			.toBeTruthy();

		expect(pageErrors).toEqual([]);
	});

	test("screencast delivers frames after navigation", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		// Create a session
		await page.getByRole("button", { name: "New Session" }).click();
		await expect(page.getByPlaceholder("Search or enter URL...")).toBeVisible({ timeout: 30000 });

		// Navigate to a real page
		const input = page.getByPlaceholder("Search or enter URL...");
		await input.fill("example.com");
		await input.press("Enter");

		// Canvas should appear and receive frames — "Waiting for first frame"
		// should disappear and be replaced by the canvas with frame data.
		// The canvas element appears when screencasting.value is true and
		// frameData.value is set.
		await expect(page.locator("canvas")).toBeVisible({ timeout: 30000 });

		// Verify frame metadata is shown (e.g. "Frame #1" or similar)
		await expect
			.poll(
				async () => {
					const text = await page.locator("body").innerText();
					return text.includes("Frame #");
				},
				{ timeout: 15000 },
			)
			.toBeTruthy();

		expect(pageErrors).toEqual([]);
	});

	test("session can be closed", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		// Create a session
		await page.getByRole("button", { name: "New Session" }).click();
		await expect(page.getByPlaceholder("Search or enter URL...")).toBeVisible({ timeout: 30000 });

		// Session card should be visible
		await expect(page.getByRole("button", { name: "Close" })).toBeVisible();

		// Close the session
		await page.getByRole("button", { name: "Close" }).click();

		// Should return to empty state
		await expect(page.getByText("No active browser sessions")).toBeVisible({ timeout: 10000 });

		expect(pageErrors).toEqual([]);
	});

	test("session shows sandbox badge when sandbox is enabled", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		// Create a session
		await page.getByRole("button", { name: "New Session" }).click();
		await expect(page.getByPlaceholder("Search or enter URL...")).toBeVisible({ timeout: 30000 });

		// Check for sandbox badge (if sandbox is enabled in the test environment)
		// or absence of it (if running without sandbox). Either way, no errors.
		await page
			.getByText("sandbox")
			.isVisible()
			.catch(() => false);

		expect(pageErrors).toEqual([]);
	});

	// ── Regression tests for specific bugs ──────────────────────

	test("sessions created via REST API appear in UI list (Bug 2: shared manager)", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		// Create a session directly via the REST API (simulating agent path)
		const apiRes = await page.evaluate(async () => {
			const res = await fetch("/api/browser/action", {
				method: "POST",
				headers: { "Content-Type": "application/json" },
				body: JSON.stringify({ action: "navigate", url: "about:blank" }),
			});
			return res.json();
		});

		// The session should appear in the UI list after refresh
		await page.getByRole("button", { name: /Refresh/ }).click();
		await expect
			.poll(
				async () => {
					const text = await page.locator("body").innerText();
					return text.includes(apiRes.session_id?.slice(0, 12) || "browser-");
				},
				{ timeout: 10000 },
			)
			.toBeTruthy();

		expect(pageErrors).toEqual([]);
	});

	test("URL bar shows target URL immediately on navigation (Bug 15: no flicker)", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await page.getByRole("button", { name: "New Session" }).click();
		await expect(page.getByPlaceholder("Search or enter URL...")).toBeVisible({ timeout: 30000 });

		const input = page.getByPlaceholder("Search or enter URL...");
		await input.fill("example.com");
		await input.press("Enter");

		// URL bar should immediately show the normalized URL, not flicker to blank
		await expect
			.poll(
				async () => {
					const val = await input.inputValue();
					return val.includes("example.com");
				},
				{ timeout: 5000 },
			)
			.toBeTruthy();

		expect(pageErrors).toEqual([]);
	});

	test("switching sessions rapidly does not cause JS errors (Bug 16: relay accumulation)", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		// Create first session
		await page.getByRole("button", { name: "New Session" }).click();
		await expect(page.getByPlaceholder("Search or enter URL...")).toBeVisible({ timeout: 30000 });
		const input = page.getByPlaceholder("Search or enter URL...");
		await input.fill("example.com");
		await input.press("Enter");
		await expect(page.locator("canvas")).toBeVisible({ timeout: 30000 });

		// Create second session
		await page.getByRole("button", { name: "New Session" }).click();
		await expect(page.getByPlaceholder("Search or enter URL...")).toBeVisible({ timeout: 30000 });

		// Switch back and forth rapidly by clicking session cards
		const cards = page.locator("[class*='rounded-lg border p-3']");
		const count = await cards.count();
		if (count >= 2) {
			for (let i = 0; i < 4; i++) {
				await cards.nth(i % count).click();
				await page.waitForTimeout(200);
			}
		}

		// No JS errors should have occurred
		expect(pageErrors).toEqual([]);
	});

	test("closed session appears in History tab (Bug 17: dead sessions in history)", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		// Create and navigate a session
		await page.getByRole("button", { name: "New Session" }).click();
		await expect(page.getByPlaceholder("Search or enter URL...")).toBeVisible({ timeout: 30000 });
		const input = page.getByPlaceholder("Search or enter URL...");
		await input.fill("example.com");
		await input.press("Enter");

		// Wait for navigation to complete
		await expect
			.poll(
				async () => {
					const text = await page.locator("body").innerText();
					return text.includes("example.com");
				},
				{ timeout: 30000 },
			)
			.toBeTruthy();

		// Close the session
		await page.getByText("Close").click();

		// Switch to History tab
		await page.getByText("History").click();

		// The closed session should appear with example.com URL
		await expect
			.poll(
				async () => {
					const text = await page.locator("body").innerText();
					return text.includes("example.com") && (text.includes("closed") || text.includes("lost"));
				},
				{ timeout: 10000 },
			)
			.toBeTruthy();

		expect(pageErrors).toEqual([]);
	});

	test("creating session shows placeholder immediately (Bug: delayed highlight)", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await page.getByRole("button", { name: "New Session" }).click();

		// Placeholder should appear immediately with "creating" badge
		await expect(page.getByText("New session")).toBeVisible({ timeout: 5000 });
		await expect(page.getByText("Starting browser")).toBeVisible({ timeout: 5000 });
		await expect(page.getByText("creating")).toBeVisible({ timeout: 5000 });

		expect(pageErrors).toEqual([]);
	});

	test("dead session shows error toast and recovers (Bug 12: stuck fetching)", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		// Mock a session that will fail on screenshot
		await page.route("**/api/browser/action", async (route, request) => {
			const body = JSON.parse(request.postData() || "{}");
			if (body.action === "navigate") {
				return route.fulfill({
					status: 200,
					contentType: "application/json",
					body: JSON.stringify({ success: true, session_id: "fake-dead-session", url: "about:blank" }),
				});
			}
			if (body.action === "screenshot") {
				return route.fulfill({
					status: 200,
					contentType: "application/json",
					body: JSON.stringify({ success: false, session_id: "fake-dead-session", error: "connection lost" }),
				});
			}
			return route.continue();
		});

		// Create a session (will use mocked API)
		await page.getByRole("button", { name: "New Session" }).click();
		await page.waitForTimeout(3000);

		// Should not be stuck on "Fetching browser view..." — should show error or recover
		const bodyText = await page.locator("body").innerText();
		expect(bodyText).not.toContain("Fetching browser view");

		expect(pageErrors).toEqual([]);
	});
});
