const { expect, test } = require("@playwright/test");
const { navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

async function setChatSeq(page, seq) {
	await page.evaluate(async (nextSeq) => {
		var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
		if (!appScript) throw new Error("app module script not found");
		var appUrl = new URL(appScript.src, window.location.origin);
		var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
		var state = await import(`${prefix}js/state.js`);
		state.setChatSeq(nextSeq);
	}, seq);
}

async function getChatSeq(page) {
	return await page.evaluate(async () => {
		var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
		if (!appScript) throw new Error("app module script not found");
		var appUrl = new URL(appScript.src, window.location.origin);
		var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
		var state = await import(`${prefix}js/state.js`);
		return state.chatSeq;
	});
}

test.describe("Chat input and slash commands", () => {
	test.beforeEach(async ({ page }) => {
		await navigateAndWait(page, "/chats/main");
		await waitForWsConnected(page);
	});

	test("chat input is visible and focusable", async ({ page }) => {
		const chatInput = page.locator("#chatInput");
		await expect(chatInput).toBeVisible();
		await chatInput.focus();
		await expect(chatInput).toBeFocused();
	});

	test('typing "/" shows slash command menu', async ({ page }) => {
		const chatInput = page.locator("#chatInput");
		await chatInput.focus();
		await chatInput.fill("/");

		const slashMenu = page.locator(".slash-menu");
		await expect(slashMenu).toBeVisible({ timeout: 5_000 });

		// Should have at least one menu item
		const items = slashMenu.locator(".slash-menu-item");
		await expect(items).not.toHaveCount(0);
	});

	test("slash menu filters as user types", async ({ page }) => {
		const chatInput = page.locator("#chatInput");
		await chatInput.focus();
		await chatInput.fill("/");

		const slashMenu = page.locator(".slash-menu");
		await expect(slashMenu).toBeVisible({ timeout: 5_000 });

		const countAll = await slashMenu.locator(".slash-menu-item").count();

		// Type more to filter
		await chatInput.fill("/cl");
		await page.waitForTimeout(200);

		const countFiltered = await slashMenu.locator(".slash-menu-item").count();
		expect(countFiltered).toBeLessThanOrEqual(countAll);
	});

	test("Escape dismisses slash menu", async ({ page }) => {
		const chatInput = page.locator("#chatInput");
		await chatInput.focus();
		await chatInput.fill("/");

		const slashMenu = page.locator(".slash-menu");
		await expect(slashMenu).toBeVisible({ timeout: 5_000 });

		await page.keyboard.press("Escape");
		await expect(slashMenu).toBeHidden();
	});

	test("Shift+Enter inserts newline without sending", async ({ page }) => {
		const chatInput = page.locator("#chatInput");
		await chatInput.focus();
		await chatInput.fill("line one");
		await page.keyboard.press("Shift+Enter");
		await page.keyboard.type("line two");

		const value = await chatInput.inputValue();
		expect(value).toContain("line one");
		expect(value).toContain("line two");
	});

	test("model selector dropdown opens and closes", async ({ page }) => {
		const modelBtn = page.locator("#modelComboBtn");
		if (await modelBtn.isVisible()) {
			await modelBtn.click();

			const dropdown = page.locator("#modelDropdown");
			await expect(dropdown).toBeVisible();

			// Close by clicking button again
			await modelBtn.click();
			await expect(dropdown).toBeHidden();
		}
	});

	test("send button is present", async ({ page }) => {
		const sendBtn = page.locator("#sendBtn");
		await expect(sendBtn).toBeVisible();
	});

	test("prompt button is hidden from chat header", async ({ page }) => {
		await expect(page.locator("#rawPromptBtn")).toHaveCount(0);
	});

	test("full context copy button uses small button style", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.locator("#fullContextBtn").click();

		const copyBtn = page.locator("#fullContextPanel button", { hasText: "Copy" });
		await expect(copyBtn).toBeVisible();
		await expect(copyBtn).toHaveClass(/provider-btn-sm/);
		expect(pageErrors).toEqual([]);
	});

	test("/clear resets client chat sequence", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await setChatSeq(page, 8);

		const chatInput = page.locator("#chatInput");
		await chatInput.fill("/clear");
		await page.keyboard.press("Enter");

		await expect.poll(async () => await getChatSeq(page)).toBe(0);
		expect(pageErrors).toEqual([]);
	});
});
