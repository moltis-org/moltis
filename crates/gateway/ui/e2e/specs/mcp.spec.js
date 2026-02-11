const { expect, test } = require("@playwright/test");
const { navigateAndWait, watchPageErrors } = require("../helpers");

test.describe("MCP page", () => {
	test("MCP page loads", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/mcp");

		await expect(page.getByRole("heading", { name: "MCP", exact: true })).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("featured servers shown", async ({ page }) => {
		await navigateAndWait(page, "/settings/mcp");

		// MCP page should display featured servers or server list
		const content = page.locator("#pageContent");
		await expect(content).not.toBeEmpty();
	});

	test("page has no JS errors", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/mcp");
		expect(pageErrors).toEqual([]);
	});
});
