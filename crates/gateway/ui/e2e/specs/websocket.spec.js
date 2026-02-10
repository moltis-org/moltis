const { expect, test } = require("@playwright/test");
const { waitForWsConnected, watchPageErrors } = require("../helpers");

test.describe("WebSocket connection lifecycle", () => {
	test("status shows connected after page load", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/");
		await waitForWsConnected(page);

		await expect(page.locator("#statusDot")).toHaveClass(/connected/);
		// When connected, statusText is intentionally cleared to ""
		await expect(page.locator("#statusText")).toHaveText("");
		expect(pageErrors).toEqual([]);
	});

	test("memory info updates from tick events", async ({ page }) => {
		await page.goto("/");
		await waitForWsConnected(page);

		// tick events carry memory stats; wait for memoryInfo to populate
		await expect(page.locator("#memoryInfo")).not.toHaveText("", {
			timeout: 15_000,
		});
	});

	test("connection persists across SPA navigation", async ({ page }) => {
		await page.goto("/");
		await waitForWsConnected(page);

		// Navigate to a different page within the SPA
		await page.goto("/settings");
		await expect(page.locator("#pageContent")).not.toBeEmpty();

		// WebSocket should remain connected through client-side navigation
		await expect(page.locator("#statusDot")).toHaveClass(/connected/);

		// Navigate back to chat
		await page.goto("/chats/main");
		await expect(page.locator("#pageContent")).not.toBeEmpty();
		await expect(page.locator("#statusDot")).toHaveClass(/connected/);
	});

	test("health endpoint responds", async ({ request }) => {
		// Verify the server is healthy via the HTTP health endpoint
		const resp = await request.get("/health");
		expect(resp.ok()).toBeTruthy();
	});
});
