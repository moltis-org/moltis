const { test, expect } = require("@playwright/test");
const { navigateAndWait, watchPageErrors } = require("../helpers");

test.describe("SSH settings", () => {
	test("can generate a deploy key and add a managed SSH target", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/ssh");
		await expect(page.locator('.settings-nav-item[data-section="ssh"]')).toHaveText("SSH");
		await expect(page.locator('.settings-nav-item[data-section="ssh"] .icon')).toHaveCount(0);

		const suffix = Date.now().toString().slice(-6);
		const keyName = `e2e-key-${suffix}`;
		const targetLabel = `e2e-target-${suffix}`;

		await page.getByPlaceholder("production-box").fill(keyName);
		await page.getByRole("button", { name: "Generate", exact: true }).click();

		await expect(page.locator(".provider-item-name", { hasText: keyName }).first()).toBeVisible({
			timeout: 15_000,
		});
		await expect(page.getByRole("button", { name: "Copy Public Key", exact: true }).first()).toBeVisible();
		const publicKey = await page.locator("pre").first().textContent();
		expect(publicKey).toContain("ssh-ed25519 ");

		// Switch to Targets tab to add a target
		await page.getByRole("tab", { name: /Targets/ }).click();
		await page.getByPlaceholder("prod-box").fill(targetLabel);
		await page.getByPlaceholder("deploy@example.com").fill("deploy@example.com");
		await page.locator("select").nth(0).selectOption("managed");
		await page
			.getByPlaceholder("Optional known_hosts line from ssh-keyscan -H host")
			.fill(`prod.example.com ${publicKey.trim()}`);
		await page.locator("select").nth(1).selectOption({ label: keyName });
		await page.getByRole("button", { name: "Add Target", exact: true }).click();

		const targetCard = page.locator(".provider-item", { hasText: targetLabel }).first();
		await expect(targetCard).toBeVisible({ timeout: 15_000 });
		await expect(targetCard.getByText("Managed key", { exact: true })).toBeVisible();
		await expect(targetCard.getByText("Host pinned", { exact: true })).toBeVisible();

		await targetCard.getByRole("button", { name: "Clear Pin", exact: true }).click();
		await expect(targetCard.getByText("Uses global known_hosts", { exact: true })).toBeVisible({
			timeout: 15_000,
		});

		expect(pageErrors).toEqual([]);
	});
});
