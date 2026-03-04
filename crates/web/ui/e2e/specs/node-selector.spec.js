const { expect, test } = require("../base-test");
const { navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

test.describe("Node selector", () => {
	test("node selector is hidden when no nodes connected", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/chats/main");
		await waitForWsConnected(page);

		const nodeCombo = page.locator("#nodeCombo");
		await expect(nodeCombo).toBeHidden();

		expect(pageErrors).toEqual([]);
	});

	test("node selector exists in chat toolbar DOM", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/chats/main");
		await waitForWsConnected(page);

		const nodeCombo = page.locator("#nodeCombo");
		await expect(nodeCombo).toHaveCount(1);

		const nodeComboBtn = page.locator("#nodeComboBtn");
		await expect(nodeComboBtn).toHaveCount(1);

		const nodeDropdown = page.locator("#nodeDropdown");
		await expect(nodeDropdown).toHaveCount(1);
		await expect(nodeDropdown).toBeHidden();

		expect(pageErrors).toEqual([]);
	});

	test("node combo label shows Local by default", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/chats/main");
		await waitForWsConnected(page);

		const label = page.locator("#nodeComboLabel");
		await expect(label).toHaveText("Local");

		expect(pageErrors).toEqual([]);
	});
});
