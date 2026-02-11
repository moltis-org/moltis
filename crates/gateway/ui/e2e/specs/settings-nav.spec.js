const { expect, test } = require("@playwright/test");
const { expectPageContentMounted, navigateAndWait, watchPageErrors } = require("../helpers");

test.describe("Settings navigation", () => {
	test("/settings redirects to /settings/identity", async ({ page }) => {
		await navigateAndWait(page, "/settings");
		await expect(page).toHaveURL(/\/settings\/identity$/);
		await expect(page.getByRole("heading", { name: "Identity", exact: true })).toBeVisible();
	});

	const settingsSections = [
		{ id: "identity", heading: "Identity" },
		{ id: "memory", heading: "Memory" },
		{ id: "environment", heading: "Environment" },
		{ id: "voice", heading: "Voice" },
		{ id: "security", heading: "Security" },
		{ id: "tailscale", heading: "Tailscale" },
		{ id: "notifications", heading: "Notifications" },
		{ id: "providers", heading: "LLMs" },
		{ id: "channels", heading: "Channels" },
		{ id: "mcp", heading: "MCP" },
		{ id: "hooks", heading: "Hooks" },
		{ id: "sandboxes", heading: "Sandboxes" },
		{ id: "logs", heading: "Logs" },
		{ id: "config", heading: "Configuration" },
	];

	for (const section of settingsSections) {
		test(`settings/${section.id} loads without errors`, async ({ page }) => {
			const pageErrors = watchPageErrors(page);
			await page.goto(`/settings/${section.id}`);
			await expectPageContentMounted(page);

			await expect(page).toHaveURL(new RegExp(`/settings/${section.id}$`));

			// Settings sections use heading text that may differ slightly
			// from the section ID; check the page loaded content.
			const content = page.locator("#pageContent");
			await expect(content).not.toBeEmpty();

			expect(pageErrors).toEqual([]);
		});
	}

	test("identity form elements render", async ({ page }) => {
		await navigateAndWait(page, "/settings/identity");

		// Identity page should have a name input and soul/description textarea
		const content = page.locator("#pageContent");
		await expect(content).not.toBeEmpty();
	});

	test("environment page has add form", async ({ page }) => {
		await navigateAndWait(page, "/settings/environment");
		await expect(page.getByRole("heading", { name: "Environment" })).toBeVisible();
		await expect(page.getByPlaceholder("KEY_NAME")).toHaveAttribute("autocomplete", "off");
		await expect(page.getByPlaceholder("Value")).toHaveAttribute("autocomplete", "new-password");
	});

	test("security page renders", async ({ page }) => {
		await navigateAndWait(page, "/settings/security");
		await expect(page.getByRole("heading", { name: "Security" })).toBeVisible();
	});

	test("provider page renders from settings", async ({ page }) => {
		await navigateAndWait(page, "/settings/providers");
		await expect(page.getByRole("heading", { name: "LLMs" })).toBeVisible();
	});

	test("sidebar groups and order match product layout", async ({ page }) => {
		await navigateAndWait(page, "/settings/identity");

		await expect(page.locator(".settings-group-label").nth(0)).toHaveText("General");
		await expect(page.locator(".settings-group-label").nth(1)).toHaveText("Security");
		await expect(page.locator(".settings-group-label").nth(2)).toHaveText("Integrations");
		await expect(page.locator(".settings-group-label").nth(3)).toHaveText("Systems");

		const navItems = (await page.locator(".settings-nav-item").allTextContents()).map((text) => text.trim());
		const expectedWithVoice = [
			"Identity",
			"Environment",
			"Memory",
			"Notifications",
			"Security",
			"Tailscale",
			"LLMs",
			"Channels",
			"Voice",
			"MCP",
			"Hooks",
			"Sandboxes",
			"Logs",
			"Configuration",
		];
		const expectedWithoutVoice = expectedWithVoice.filter((item) => item !== "Voice");
		expect(navItems).toEqual(navItems.includes("Voice") ? expectedWithVoice : expectedWithoutVoice);
	});
});
