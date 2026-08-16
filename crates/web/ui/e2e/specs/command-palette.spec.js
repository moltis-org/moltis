const { expect, test } = require("../base-test");
const { expectPageContentMounted, navigateAndWait, watchPageErrors } = require("../helpers");

test.describe("Command palette", () => {
	test("opens on Ctrl+K and closes on Escape", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/");

		await expect(page.locator(".cmd-palette")).toHaveCount(0);

		await page.keyboard.press("Control+k");
		await expect(page.locator(".cmd-palette")).toBeVisible();
		await expect(page.locator(".cmd-palette-input")).toBeVisible();

		await page.keyboard.press("Escape");
		await expect(page.locator(".cmd-palette")).toHaveCount(0);

		expect(pageErrors).toEqual([]);
	});

	test("opens on Ctrl+K for non-Mac platforms", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/");

		await page.keyboard.press("Control+k");
		await expect(page.locator(".cmd-palette")).toBeVisible();

		await page.keyboard.press("Escape");
		expect(pageErrors).toEqual([]);
	});

	test("opens on header button click", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/");

		const btn = page.locator("#commandPaletteBtn");
		await expect(btn).toBeVisible();
		await btn.click();
		await expect(page.locator(".cmd-palette")).toBeVisible();
		await expect(page.locator(".cmd-palette-input")).toBeFocused();

		await page.keyboard.press("Escape");
		expect(pageErrors).toEqual([]);
	});

	test("closes on backdrop click", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/");

		await page.keyboard.press("Control+k");
		await expect(page.locator(".cmd-palette")).toBeVisible();

		// Click the backdrop well below the dialog to avoid edge/DPR issues
		const backdrop = page.locator(".cmd-palette-backdrop");
		const box = await backdrop.boundingBox();
		await backdrop.click({ position: { x: 10, y: box.height - 20 } });
		await expect(page.locator(".cmd-palette")).toHaveCount(0);

		expect(pageErrors).toEqual([]);
	});

	test("toggle: second Ctrl+K closes the palette", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/");

		await page.keyboard.press("Control+k");
		await expect(page.locator(".cmd-palette")).toBeVisible();

		await page.keyboard.press("Control+k");
		await expect(page.locator(".cmd-palette")).toHaveCount(0);

		expect(pageErrors).toEqual([]);
	});

	test("shows grouped commands by default", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/");

		await page.keyboard.press("Control+k");
		await expect(page.locator(".cmd-palette")).toBeVisible();

		// Should show group headers
		await expect(page.locator(".cmd-palette-group", { hasText: "Navigation" })).toBeVisible();
		await expect(page.locator(".cmd-palette-group", { hasText: "Settings" })).toBeVisible();
		await expect(page.locator(".cmd-palette-group", { hasText: "Actions" })).toBeVisible();

		// Should show some commands
		await expect(page.locator(".cmd-palette-item", { hasText: "Chats" })).toBeVisible();
		await expect(page.locator(".cmd-palette-item", { hasText: "New Session" })).toBeVisible();

		await page.keyboard.press("Escape");
		expect(pageErrors).toEqual([]);
	});

	test("filters commands by typing", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/");

		await page.keyboard.press("Control+k");
		await expect(page.locator(".cmd-palette")).toBeVisible();

		await page.locator(".cmd-palette-input").fill("prov");

		// Should show Provider-related items
		await expect(page.locator(".cmd-palette-item", { hasText: "Providers" })).toBeVisible();

		// Navigation items that don't match should be gone
		await expect(page.locator(".cmd-palette-item", { hasText: "Crons" })).toHaveCount(0);

		await page.keyboard.press("Escape");
		expect(pageErrors).toEqual([]);
	});

	test("keeps Ask agent selected when pending session results arrive", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/");
		await page.evaluate(() => {
			window.__paletteSessionSearchId = null;
			const originalSend = WebSocket.prototype.send;
			WebSocket.prototype.send = function (data) {
				try {
					const request = JSON.parse(data);
					if (request?.method === "sessions.search") {
						window.__paletteSessionSearchId = request.id;
						return;
					}
				} catch {
					// Pass non-JSON WebSocket traffic through unchanged.
				}
				return originalSend.call(this, data);
			};
		});

		await page.keyboard.press("Control+k");
		await page.getByRole("dialog", { name: "Command palette" }).getByRole("textbox").fill("xyznonexistent");

		const askAgent = page.getByRole("option", { name: /Ask agent/ });
		await expect(askAgent).toBeVisible();
		await expect(askAgent).toContainText("xyznonexistent");
		await expect(askAgent).toHaveAttribute("aria-selected", "true");
		await expect.poll(() => page.evaluate(() => window.__paletteSessionSearchId)).not.toBeNull();
		await page.evaluate(async () => {
			const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			if (!appScript) throw new Error("app module script not found");
			const appUrl = new URL(appScript.src, window.location.origin);
			const prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			const state = await import(`${prefix}js/state.js`);
			const id = window.__paletteSessionSearchId;
			const resolver = state.pending?.[id];
			if (typeof resolver !== "function") throw new Error("sessions.search resolver not found");
			delete state.pending[id];
			resolver({
				ok: true,
				payload: [{ sessionKey: "main", label: "Existing session", snippet: "xyznonexistent" }],
			});
		});

		await expect(page.getByRole("option", { name: /Existing session/ })).toBeVisible();
		await expect(askAgent).toHaveAttribute("aria-selected", "true");
		await page.keyboard.press("Enter");

		await expect(page).toHaveURL(/\/chats\/session\/[0-9a-f-]+$/);
		await expect(page.getByText("xyznonexistent", { exact: true })).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("asks the agent in a new session from the fallback", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/skills");
		await page.evaluate(() => {
			window.__paletteChatSendPayloads = [];
			const originalSend = WebSocket.prototype.send;
			WebSocket.prototype.send = function (data) {
				try {
					const request = JSON.parse(data);
					if (request?.method === "chat.send") {
						window.__paletteChatSendPayloads.push(request.params || {});
						return;
					}
				} catch {
					// Pass non-JSON WebSocket traffic through unchanged.
				}
				return originalSend.call(this, data);
			};
		});

		await page.keyboard.press("Control+k");
		await page.getByRole("dialog", { name: "Command palette" }).getByRole("textbox").fill("Plan a weekend in Lisbon");
		await expect(page.getByRole("option", { name: /Ask agent/ })).toBeVisible();
		await page.keyboard.press("Enter");

		await expect(page.getByText("Plan a weekend in Lisbon", { exact: true })).toBeVisible();
		await expect(page).toHaveURL(/\/chats\/session\/[0-9a-f-]+$/);
		await expect
			.poll(() =>
				page.evaluate(() => {
					const payloads = window.__paletteChatSendPayloads || [];
					return payloads[payloads.length - 1] || null;
				}),
			)
			.toMatchObject({ text: "Plan a weekend in Lisbon" });
		const payload = await page.evaluate(() => window.__paletteChatSendPayloads.at(-1));
		expect(payload._session_key).toMatch(/^session:[0-9a-f-]+$/);
		expect(pageErrors).toEqual([]);
	});

	test("keyword search finds commands", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/");

		await page.keyboard.press("Control+k");
		await page.locator(".cmd-palette-input").fill("docker");

		// "docker" is a keyword for Sandboxes
		await expect(page.locator(".cmd-palette-item", { hasText: "Sandboxes" })).toBeVisible();

		await page.keyboard.press("Escape");
		expect(pageErrors).toEqual([]);
	});

	test("keyboard navigation with ArrowDown/Up and Enter", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/");

		await page.keyboard.press("Control+k");
		await expect(page.locator(".cmd-palette")).toBeVisible();

		// First item should be active by default
		const firstItem = page.locator(".cmd-palette-item").first();
		await expect(firstItem).toHaveClass(/cmd-palette-item-active/);

		// Arrow down moves to second item
		await page.keyboard.press("ArrowDown");
		const secondItem = page.locator(".cmd-palette-item").nth(1);
		await expect(secondItem).toHaveClass(/cmd-palette-item-active/);
		await expect(firstItem).not.toHaveClass(/cmd-palette-item-active/);

		// Arrow up moves back
		await page.keyboard.press("ArrowUp");
		await expect(firstItem).toHaveClass(/cmd-palette-item-active/);

		await page.keyboard.press("Escape");
		expect(pageErrors).toEqual([]);
	});

	test("Enter on a navigation command navigates to the page", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/");

		await page.keyboard.press("Control+k");
		await page.locator(".cmd-palette-input").fill("Skills");

		// Wait for the filtered result to become the active command before pressing Enter.
		await expect(page.locator(".cmd-palette-item-active .cmd-palette-item-label")).toHaveText("Skills");

		await page.keyboard.press("Enter");

		// Palette should close
		await expect(page.locator(".cmd-palette")).toHaveCount(0);

		// Should navigate to skills page
		await expect(page).toHaveURL(/\/skills$/, { timeout: 10_000 });
		await expectPageContentMounted(page);

		expect(pageErrors).toEqual([]);
	});

	test("clicking a command item navigates and closes palette", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/");

		await page.keyboard.press("Control+k");
		await page.locator(".cmd-palette-input").fill("Logs");

		const logsItem = page.locator(".cmd-palette-item", { hasText: "Logs" }).first();
		await expect(logsItem).toBeVisible();
		await logsItem.click();

		await expect(page.locator(".cmd-palette")).toHaveCount(0);
		await expect(page).toHaveURL(/\/logs$|\/settings\/logs$/, { timeout: 10_000 });

		expect(pageErrors).toEqual([]);
	});

	test("mouse hover updates active item", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/");

		await page.keyboard.press("Control+k");
		await expect(page.locator(".cmd-palette")).toBeVisible();

		const thirdItem = page.locator(".cmd-palette-item").nth(2);
		await thirdItem.hover();
		await expect(thirdItem).toHaveClass(/cmd-palette-item-active/);

		await page.keyboard.press("Escape");
		expect(pageErrors).toEqual([]);
	});

	test("resets query and active index when reopened", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/");

		// Open and type a query
		await page.keyboard.press("Control+k");
		await page.locator(".cmd-palette-input").fill("test");
		await page.keyboard.press("ArrowDown");
		await page.keyboard.press("ArrowDown");
		await page.keyboard.press("Escape");

		// Reopen — should be reset
		await page.keyboard.press("Control+k");
		await expect(page.locator(".cmd-palette-input")).toHaveValue("");
		const firstItem = page.locator(".cmd-palette-item").first();
		await expect(firstItem).toHaveClass(/cmd-palette-item-active/);

		await page.keyboard.press("Escape");
		expect(pageErrors).toEqual([]);
	});

	test("header button shows keyboard shortcut badge", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/");

		const kbd = page.locator("#cmdPaletteKbd");
		await expect(kbd).toBeVisible();
		const text = await kbd.textContent();
		// Should be either ⌘K or Ctrl+K
		expect(text === "\u2318K" || text === "Ctrl+K").toBeTruthy();

		expect(pageErrors).toEqual([]);
	});

	test("mobile: header kbd badge is hidden", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.setViewportSize({ width: 390, height: 844 });
		await navigateAndWait(page, "/");

		const kbd = page.locator("#cmdPaletteKbd");
		await expect(kbd).toBeHidden();

		expect(pageErrors).toEqual([]);
	});

	test("palette has correct ARIA roles", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/");

		await page.keyboard.press("Control+k");
		await expect(page.locator(".cmd-palette")).toBeVisible();

		await expect(page.locator('[role="dialog"][aria-modal="true"]')).toBeVisible();
		await expect(page.locator('[role="listbox"]')).toBeVisible();
		const firstOption = page.locator('[role="option"]').first();
		await expect(firstOption).toBeVisible();

		await page.keyboard.press("Escape");
		expect(pageErrors).toEqual([]);
	});
});
