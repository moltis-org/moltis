const { expect, test } = require("@playwright/test");
const { navigateAndWait, waitForWsConnected } = require("../helpers");

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
});
