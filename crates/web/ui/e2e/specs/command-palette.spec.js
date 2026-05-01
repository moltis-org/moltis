const { expect, test } = require("../base-test");
const { expectPageContentMounted, navigateAndWait, watchPageErrors } = require("../helpers");

test.describe("Command palette", () => {
	test("opens on Ctrl+K and closes on Escape", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/");

		await expect(page.locator(".cmd-palette")).toHaveCount(0);

		await page.keyboard.press("Control+k");
		await expect(page.locator(".cmd-palette")).toBeVisible();
		// Focus is set in useLayoutEffect; wait for it to settle
		await expect
			.poll(() => page.locator(".cmd-palette-input").evaluate((el) => el === document.activeElement))
			.toBeTruthy();

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

	test("shows no matches when nothing found", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/");

		await page.keyboard.press("Control+k");
		await page.locator(".cmd-palette-input").fill("xyznonexistent");

		await expect(page.locator(".cmd-palette-empty")).toBeVisible();

		await page.keyboard.press("Escape");
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

		// Wait for filtered results
		await expect(page.locator(".cmd-palette-item", { hasText: "Skills" }).first()).toBeVisible();

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
