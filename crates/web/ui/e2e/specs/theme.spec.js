const { expect, test } = require("@playwright/test");
const { navigateAndWait } = require("../helpers");

test.describe("Theme toggle", () => {
	test("theme toggle buttons are visible", async ({ page }) => {
		await navigateAndWait(page, "/");

		const themeToggle = page.locator("#themeToggle");
		await expect(themeToggle).toBeVisible();

		// Should have theme buttons (light, dark, system)
		const buttons = themeToggle.locator(".theme-btn");
		const count = await buttons.count();
		expect(count).toBeGreaterThanOrEqual(2);
	});

	test("dark theme applies data-theme attribute", async ({ page }) => {
		await navigateAndWait(page, "/");

		// Find and click the dark theme button
		const darkBtn = page.locator('#themeToggle .theme-btn[data-theme="dark"]');
		if (await darkBtn.isVisible()) {
			await darkBtn.click();
		} else {
			// Fallback: try finding by aria-label or title
			const fallback = page.locator("#themeToggle .theme-btn").last();
			await fallback.click();
		}

		await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
	});

	test("theme persists across page reload", async ({ page }) => {
		await navigateAndWait(page, "/");

		// Set dark theme
		const darkBtn = page.locator('#themeToggle .theme-btn[data-theme="dark"]');
		if (await darkBtn.isVisible()) {
			await darkBtn.click();
		} else {
			const fallback = page.locator("#themeToggle .theme-btn").last();
			await fallback.click();
		}

		await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");

		// Reload the page
		await page.reload();
		await expect(page.locator("#pageContent")).not.toBeEmpty();

		// Theme should persist from localStorage
		const theme = await page.evaluate(() => document.documentElement.getAttribute("data-theme"));
		expect(theme).toBe("dark");
	});
});
