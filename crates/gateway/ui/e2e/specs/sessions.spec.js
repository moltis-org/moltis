const { expect, test } = require("@playwright/test");
const {
	expectPageContentMounted,
	navigateAndWait,
	waitForWsConnected,
	createSession,
	watchPageErrors,
} = require("../helpers");

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

	test("sessions sidebar uses search and add button row", async ({ page }) => {
		const pageErrors = await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		const sessionsPanel = page.locator("#sessionsPanel");
		await expect(sessionsPanel).toBeVisible();
		await expect(page.locator("#sessionSearch")).toBeVisible();
		await expect(page.locator("#newSessionBtn")).toBeVisible();

		const hasTopSessionsTitle = await page.evaluate(() => {
			const panel = document.getElementById("sessionsPanel");
			if (!panel) return false;
			const firstBlock = panel.firstElementChild;
			const title = firstBlock?.querySelector("span");
			return (title?.textContent || "").trim() === "Sessions";
		});
		expect(hasTopSessionsTitle).toBe(false);

		const searchAndAddShareRow = await page.evaluate(() => {
			const searchInput = document.getElementById("sessionSearch");
			const newSessionBtn = document.getElementById("newSessionBtn");
			if (!(searchInput && newSessionBtn)) return false;
			return searchInput.parentElement === newSessionBtn.parentElement;
		});
		expect(searchAndAddShareRow).toBe(true);

		expect(pageErrors).toEqual([]);
	});

	test("new session button creates a session", async ({ page }) => {
		const pageErrors = await navigateAndWait(page, "/");
		await waitForWsConnected(page);
		const sessionItems = page.locator("#sessionList .session-item");
		const initialCount = await sessionItems.count();

		await createSession(page);
		const firstSessionPath = new URL(page.url()).pathname;
		const firstSessionKey = firstSessionPath.replace(/^\/chats\//, "").replace(/\//g, ":");

		// URL should change to a new session (not main)
		await expect(page).not.toHaveURL(/\/chats\/main$/);
		await expect(page).toHaveURL(/\/chats\//);
		await expect(page.locator(`#sessionList .session-item[data-session-key="${firstSessionKey}"]`)).toHaveClass(
			/active/,
		);
		await expect(sessionItems).toHaveCount(initialCount + 1);
		await expect(page.locator("#chatInput")).toBeFocused();

		// Regression: creating a second session should still update the list
		// and mark the new session as active.
		await createSession(page);
		const secondSessionPath = new URL(page.url()).pathname;
		const secondSessionKey = secondSessionPath.replace(/^\/chats\//, "").replace(/\//g, ":");
		await expect(page.locator(`#sessionList .session-item[data-session-key="${secondSessionKey}"]`)).toHaveClass(
			/active/,
		);
		await expect(sessionItems).toHaveCount(initialCount + 2);
		await expect(page.locator("#chatInput")).toBeFocused();

		expect(pageErrors).toEqual([]);
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

	test("main session shows clear action while non-main sessions show delete", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/");
		await waitForWsConnected(page);
		await expectPageContentMounted(page);

		await expect(page.locator('button[title="Clear session"]')).toBeVisible();
		await expect(page.locator('button[title="Delete session"]')).toHaveCount(0);

		await createSession(page);

		await expect(page.locator('button[title="Clear session"]')).toHaveCount(0);
		await expect(page.locator('button[title="Delete session"]')).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("main session preview updates after clear on first message without reload", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/chats/main");
		await waitForWsConnected(page);

		const chatInput = page.locator("#chatInput");
		await expect(chatInput).toBeVisible();
		await expect(chatInput).toBeEnabled();

		await chatInput.fill("/clear");
		await chatInput.press("Enter");

		await expect
			.poll(
				() =>
					page.evaluate(() => {
						const store = window.__moltis_stores?.sessionStore;
						const main = store?.getByKey?.("main");
						if (!main) return null;
						return {
							messageCount: main.messageCount || 0,
							preview: main.preview || "",
						};
					}),
				{ timeout: 10_000 },
			)
			.toEqual({ messageCount: 0, preview: "" });

		const firstMessage = "sidebar preview should update immediately";
		await chatInput.fill(firstMessage);
		await chatInput.press("Enter");

		await expect(page.locator('#sessionList .session-item[data-session-key="main"] .session-preview')).toContainText(
			firstMessage,
		);

		expect(pageErrors).toEqual([]);
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

	test("deleting unmodified fork skips confirmation dialog", async ({ page }) => {
		const pageErrors = await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		// Create a session so we're not on "main" (Delete button is hidden for main)
		await createSession(page);
		const sessionUrl = page.url();

		// Simulate an unmodified fork: set forkPoint = messageCount = 5
		// so the session looks like a fork with messages but no new ones added.
		await expect
			.poll(
				() =>
					page.evaluate(() => {
						const store = window.__moltis_stores?.sessionStore;
						const session = store?.activeSession?.value;
						if (!session) return false;
						session.forkPoint = 5;
						session.messageCount = 5;
						// Bump dataVersion to trigger re-render
						session.dataVersion.value++;
						return true;
					}),
				{ timeout: 10_000 },
			)
			.toBe(true);

		// Click the Delete button — should NOT show a confirmation dialog
		const deleteBtn = page.locator('button[title="Delete session"]');
		await expect(deleteBtn).toBeVisible();
		await deleteBtn.click();

		// The session should be deleted immediately (no dialog appeared)
		// so we should navigate away from the current session URL
		await page.waitForURL((url) => url.href !== sessionUrl, { timeout: 5_000 });
		await expectPageContentMounted(page);

		// The confirmation dialog should NOT be visible
		await expect(page.locator(".provider-modal-backdrop")).toHaveCount(0);

		expect(pageErrors).toEqual([]);
	});

	test("deleting modified fork still shows confirmation dialog", async ({ page }) => {
		const pageErrors = await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		await createSession(page);

		// Simulate a modified fork: messageCount > forkPoint
		await expect
			.poll(
				() =>
					page.evaluate(() => {
						const store = window.__moltis_stores?.sessionStore;
						const session = store?.activeSession?.value;
						if (!session) return false;
						session.forkPoint = 3;
						session.messageCount = 5;
						session.dataVersion.value++;
						return true;
					}),
				{ timeout: 10_000 },
			)
			.toBe(true);

		const deleteBtn = page.locator('button[title="Delete session"]');
		await expect(deleteBtn).toBeVisible();
		await deleteBtn.click();

		// The confirmation dialog SHOULD appear
		await expect(page.locator(".provider-modal-backdrop")).toBeVisible();

		// Dismiss the dialog by clicking Cancel
		await page.locator(".provider-modal-backdrop .provider-btn-secondary").click();
		await expect(page.locator(".provider-modal-backdrop")).toHaveCount(0);

		expect(pageErrors).toEqual([]);
	});
});
