const { expect, test } = require("../base-test");
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

	test("request timeout setting is shown", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/mcp");

		const requestTimeoutSection = page
			.getByRole("heading", { name: "Request Timeout", exact: true })
			.locator("..")
			.locator("..");
		const timeoutInput = requestTimeoutSection.locator('input[type="number"][min="1"]');
		await expect(requestTimeoutSection.getByRole("heading", { name: "Request Timeout", exact: true })).toBeVisible();
		await expect(
			requestTimeoutSection.getByText("Controls how long Moltis waits for an MCP server response", { exact: false }),
		).toBeVisible();
		await expect(timeoutInput).toBeVisible();
		expect(Number.parseInt(await timeoutInput.inputValue(), 10)).toBeGreaterThan(0);
		expect(pageErrors).toEqual([]);
	});

	test("custom form supports remote SSE URL flow with header guidance", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/mcp");

		await page.getByRole("button", { name: "SSE (remote)", exact: true }).click();
		await expect(page.getByPlaceholder("https://mcp.linear.app/mcp")).toBeVisible();
		await expect(page.getByPlaceholder("Authorization=Bearer ...")).toBeVisible();
		await expect(page.getByText("Request headers (optional, KEY=VALUE per line)", { exact: true })).toBeVisible();
		await expect(page.getByText("Stored header values stay hidden", { exact: false })).toBeVisible();
		await expect(page.getByPlaceholder("Use global default")).toBeVisible();
		await expect(page.getByText("If the server requires OAuth", { exact: false })).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("custom form supports Streamable HTTP transport option", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/mcp");

		await page.getByRole("button", { name: "Streamable HTTP", exact: true }).click();
		await expect(page.getByPlaceholder("https://mcp.linear.app/mcp")).toBeVisible();
		await expect(page.getByPlaceholder("Authorization=Bearer ...")).toBeVisible();
		await expect(page.getByText("Request headers (optional, KEY=VALUE per line)", { exact: true })).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("configured remote server edit form shows sanitized metadata only", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.route("**/api/mcp", async (route) => {
			await route.fulfill({
				status: 200,
				contentType: "application/json",
				body: JSON.stringify([
					{
						name: "demo-remote",
						state: "stopped",
						enabled: true,
						tool_count: 0,
						transport: "sse",
						url: "https://mcp.example.com/mcp?token=[REDACTED]",
						header_names: ["Authorization", "X-Workspace"],
					},
				]),
			});
		});

		await navigateAndWait(page, "/settings/mcp");
		await expect(page.getByText("demo-remote", { exact: true })).toBeVisible();

		await page.getByRole("button", { name: "Edit", exact: true }).click();

		await expect(page.getByText("Current URL", { exact: true })).toBeVisible();
		await expect(page.getByText("https://mcp.example.com/mcp?token=[REDACTED]", { exact: true })).toBeVisible();
		await expect(page.getByText("Authorization, X-Workspace (2 total)", { exact: true })).toBeVisible();
		await expect(page.getByRole("button", { name: "Clear stored headers", exact: true })).toBeVisible();
		await expect(page.getByText("Leave blank to preserve stored headers.", { exact: false })).toBeVisible();
		await expect(page.getByText("secret-value", { exact: false })).toHaveCount(0);
		await expect(page.getByText("team-secret", { exact: false })).toHaveCount(0);
		await expect(page.getByText("top-secret", { exact: false })).toHaveCount(0);
		expect(pageErrors).toEqual([]);
	});

	test("page has no JS errors", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/mcp");
		expect(pageErrors).toEqual([]);
	});
});
