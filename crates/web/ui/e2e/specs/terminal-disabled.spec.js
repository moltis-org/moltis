const { expect, test } = require("../base-test");
const { navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

test.describe("Terminal disabled state", () => {
	test("shows disabled message when terminal_enabled is false in gon", async ({ page }) => {
		var errors = watchPageErrors(page);

		// Intercept the server's inline `window.__MOLTIS__ = {...}` assignment and
		// force terminal_enabled off while preserving the rest of the gon payload.
		await page.addInitScript(() => {
			var gonValue = { terminal_enabled: false };
			Object.defineProperty(window, "__MOLTIS__", {
				configurable: true,
				get() {
					return gonValue;
				},
				set(value) {
					gonValue = { ...(value || {}), terminal_enabled: false };
				},
			});
		});

		await navigateAndWait(page, "/settings/terminal");
		await waitForWsConnected(page);

		// The disabled message should be visible.
		await expect(page.getByText(/host terminal has been disabled/i)).toBeVisible({ timeout: 10_000 });

		// The xterm container should NOT be present.
		await expect(page.locator("#terminalOutput")).not.toBeVisible();

		expect(errors).toHaveLength(0);
	});

	test("shows terminal when terminal_enabled is true in gon", async ({ page }) => {
		var errors = watchPageErrors(page);

		await navigateAndWait(page, "/settings/terminal");
		await waitForWsConnected(page);

		// The disabled message should NOT be visible (terminal_enabled defaults to true).
		await expect(page.getByText(/host terminal has been disabled/i)).not.toBeVisible();

		// The terminal container or status should be present.
		await expect(page.locator(".terminal-page")).toBeVisible({ timeout: 10_000 });

		expect(errors).toHaveLength(0);
	});
});
