const { expect, test } = require("../base-test");
const { navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

test.describe("Signal channel", () => {
	test("connect button visible when signal is offered", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const addButton = page.getByRole("button", { name: "Connect Signal", exact: true });
		await expect(addButton).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("connect button renders a signal icon mask", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const addButton = page.getByRole("button", { name: "Connect Signal", exact: true });
		await expect(addButton).toBeVisible();

		const icon = addButton.locator(".icon.icon-signal");
		await expect(icon).toBeVisible();
		await expect
			.poll(() =>
				icon.evaluate((node) => {
					const style = window.getComputedStyle(node);
					return style.maskImage || style.webkitMaskImage || "";
				}),
			)
			.not.toBe("none");

		expect(pageErrors).toEqual([]);
	});

	test("add modal has daemon, account, and policy fields", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		await page.getByRole("button", { name: "Connect Signal", exact: true }).click();
		await expect(page.getByRole("heading", { name: "Connect Signal", exact: true })).toBeVisible();

		const accountIdInput = page.locator('input[data-field="accountId"]');
		await expect(accountIdInput).toBeVisible();

		const accountInput = page.locator('input[data-field="account"]');
		await expect(accountInput).toBeVisible();
		await expect(accountInput).toHaveAttribute("placeholder", "e.g. +15551234567");

		const httpUrlInput = page.locator('input[data-field="httpUrl"]');
		await expect(httpUrlInput).toBeVisible();
		await expect(httpUrlInput).toHaveValue("http://127.0.0.1:8080");

		const dmPolicySelect = page.locator('select[data-field="dmPolicy"]');
		await expect(dmPolicySelect).toBeVisible();
		await expect(dmPolicySelect).toHaveValue("allowlist");

		const groupPolicySelect = page.locator('select[data-field="groupPolicy"]');
		await expect(groupPolicySelect).toBeVisible();
		await expect(groupPolicySelect).toHaveValue("disabled");

		await expect(page.getByText("How to set up Signal")).toBeVisible();
		await expect(page.getByText("Run signal-cli daemon with JSON-RPC HTTP enabled")).toBeVisible();

		expect(pageErrors).toEqual([]);
	});
});
