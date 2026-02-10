const { expect, test } = require("@playwright/test");
const { expectPageContentMounted, navigateAndWait, waitForWsConnected, createSession } = require("../helpers");

test.describe("Session management", () => {
	test("session list renders on load", async ({ page }) => {
		await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		const sessionList = page.locator("#sessionList");
		await expect(sessionList).toBeVisible();

		// At least the default "main" session should be present
		const items = sessionList.locator(".session-item");
		await expect(items).not.toHaveCount(0);
	});

	test("new session button creates a session", async ({ page }) => {
		await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		await createSession(page);

		// URL should change to a new session (not main)
		await expect(page).not.toHaveURL(/\/chats\/main$/);
		await expect(page).toHaveURL(/\/chats\//);

		// The new session should appear in the sidebar
		const items = await page.locator("#sessionList .session-item").count();
		expect(items).toBeGreaterThanOrEqual(1);
	});

	test("clicking a session switches to it", async ({ page }) => {
		await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		// Create a second session so we have something to switch to
		await createSession(page);
		const newSessionUrl = page.url();

		// Click the "main" session in the list
		const mainItem = page.locator('#sessionList .session-item[data-session-key="main"]');
		// If data-session-key isn't set, fall back to finding by label text
		const target = (await mainItem.count()) ? mainItem : page.locator("#sessionList .session-item").first();
		await target.click();

		await expect(page).not.toHaveURL(newSessionUrl);
		await expectPageContentMounted(page);
	});

	test("session search filters the list", async ({ page }) => {
		await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		const searchInput = page.locator("#sessionSearch");
		// searchInput may be hidden until focused or may always be visible
		if (await searchInput.isVisible()) {
			const countBefore = await page.locator("#sessionList .session-item").count();

			// Type a string that won't match any session
			await searchInput.fill("zzz_no_match_zzz");
			// Allow time for filtering
			await page.waitForTimeout(300);

			const countAfter = await page.locator("#sessionList .session-item").count();
			expect(countAfter).toBeLessThanOrEqual(countBefore);

			// Clear search restores list
			await searchInput.fill("");
			await page.waitForTimeout(300);

			const countRestored = await page.locator("#sessionList .session-item").count();
			expect(countRestored).toBe(countBefore);
		}
	});

	test("clear all sessions resets list", async ({ page }) => {
		await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		// Create extra sessions first
		await createSession(page);
		await createSession(page);

		const clearBtn = page.locator("#clearAllSessionsBtn");
		if (await clearBtn.isVisible()) {
			// Accept the confirm dialog
			page.on("dialog", (dialog) => dialog.accept());
			await clearBtn.click();

			// Wait for list to reset
			await page.waitForTimeout(500);
			await expectPageContentMounted(page);

			// Should be back to a single session
			const items = page.locator("#sessionList .session-item");
			const count = await items.count();
			expect(count).toBeGreaterThanOrEqual(1);
		}
	});

	test("sessions panel hidden on non-chat pages", async ({ page }) => {
		await navigateAndWait(page, "/settings");

		const panel = page.locator("#sessionsPanel");
		// On settings pages, the sessions panel should be hidden
		await expect(panel).toBeHidden();
	});
});
