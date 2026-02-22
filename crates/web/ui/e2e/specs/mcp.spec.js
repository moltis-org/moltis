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

	test("linear remote server is available in featured list", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/mcp");

		await expect(page.getByRole("heading", { name: "Popular MCP Servers", exact: true })).toBeVisible();
		await expect(page.getByText("linear", { exact: true })).toBeVisible();
		await expect(page.getByText("sse remote")).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("custom form supports remote SSE URL flow", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/mcp");

		await page.getByRole("button", { name: "SSE (remote)", exact: true }).click();
		await expect(page.getByPlaceholder("https://mcp.linear.app/mcp")).toBeVisible();
		await expect(page.getByText("If the server requires OAuth", { exact: false })).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("page has no JS errors", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/mcp");
		expect(pageErrors).toEqual([]);
	});
});
