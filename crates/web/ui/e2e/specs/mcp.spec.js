const { expect, test } = require("../base-test");
const { navigateAndWait, watchPageErrors, waitForWsConnected } = require("../helpers");
const { fork } = require("node:child_process");
const path = require("node:path");

/**
 * Start the mock MCP Streamable HTTP server as a child process.
 * Returns { port, process } — caller must kill the process on teardown.
 */
function startMockMcpServer(args = []) {
	return new Promise((resolve, reject) => {
		var serverPath = path.resolve(__dirname, "../mock-mcp-server.js");
		var child = fork(serverPath, args, { silent: true });
		var output = "";
		var timeoutHandle = setTimeout(() => reject(new Error("Mock MCP server startup timeout")), 5000);

		child.stderr.on("data", (chunk) => {
			process.stderr.write(`[mock-mcp-server] ${chunk}`);
		});

		child.stdout.on("data", (chunk) => {
			output += chunk.toString();
			try {
				var parsed = JSON.parse(output.trim());
				if (parsed.port) {
					clearTimeout(timeoutHandle);
					resolve({ port: parsed.port, process: child });
				}
			} catch {
				// Not complete JSON yet, keep accumulating
			}
		});

		child.on("error", (err) => {
			clearTimeout(timeoutHandle);
			reject(err);
		});
		child.on("exit", (code) => {
			if (!output) {
				clearTimeout(timeoutHandle);
				reject(new Error(`Mock MCP server exited with code ${code}`));
			}
		});
	});
}

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

	// Issue #732: MCP status shows "dead" for working Streamable HTTP servers
	// with Bearer token auth because is_alive() health check sends GET and
	// only accepts 2xx/401 as alive. Many real servers return 405 for GET.
	//
	// This test adds a real mock MCP server via the UI form, then verifies
	// the status indicator. The mock returns 405 for GET (reproducing #732).
	test.describe("Streamable HTTP status (#732)", () => {
		var mockMcp;

		test.beforeAll(async () => {
			mockMcp = await startMockMcpServer(["--bearer-token", "e2e-test-token"]);
		});

		test.afterAll(() => {
			if (mockMcp?.process) mockMcp.process.kill();
		});

		test("server added via Streamable HTTP with Bearer token shows running status", async ({ page }) => {
			var pageErrors = watchPageErrors(page);
			await navigateAndWait(page, "/settings/mcp");
			await waitForWsConnected(page);

			// Fill out the Streamable HTTP custom server form
			await page.getByRole("button", { name: "Streamable HTTP", exact: true }).click();
			await page.getByPlaceholder("https://mcp.linear.app/mcp").fill(`http://127.0.0.1:${mockMcp.port}`);
			await page.getByPlaceholder("Authorization=Bearer ...").fill(`Authorization=Bearer e2e-test-token`);

			// Submit the form (press Enter or click Add)
			var addBtn = page.getByRole("button", { name: "Add", exact: true });
			await addBtn.click();

			// Wait for success toast
			await expect(page.getByText("Added MCP tool", { exact: false })).toBeVisible({ timeout: 15_000 });

			// The server should now appear in the list. Wait for it.
			// The name is derived from the URL, typically "127-0-0-1" or similar.
			// Navigate to refresh the MCP list (status_all is called on page load).
			await navigateAndWait(page, "/settings/mcp");

			// Find the server entry and check its status badge.
			// The mock server responds to POST (tools work) but returns 405 for GET.
			// After the fix for #732, is_alive() treats any HTTP response as alive.
			var serverEntry = page.locator(".skills-repo-card").filter({
				has: page.locator(`text=127-0-0-1`),
			});
			await expect(serverEntry).toBeVisible({ timeout: 10_000 });

			// Check the state badge text — should be "running" now that #732 is fixed.
			// Use toHaveText with retry to avoid flakes during status transitions.
			var stateBadge = serverEntry.locator("span").filter({ hasText: /^(running|dead|stopped|connecting)$/ }).first();
			await expect(stateBadge).toHaveText("running", { timeout: 15_000 });

			// Verify the server has 1 tool (mock_echo) — proves POST connection worked
			await expect(serverEntry.getByText("1 tool", { exact: false })).toBeVisible();

			expect(pageErrors).toEqual([]);
		});
	});
});
