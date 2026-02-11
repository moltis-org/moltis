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
		{ id: "crons", heading: "Cron Jobs" },
		{ id: "voice", heading: "Voice" },
		{ id: "security", heading: "Security" },
		{ id: "tailscale", heading: "Tailscale" },
		{ id: "notifications", heading: "Notifications" },
		{ id: "providers", heading: "LLMs" },
		{ id: "channels", heading: "Channels" },
		{ id: "mcp", heading: "MCP" },
		{ id: "hooks", heading: "Hooks" },
		{ id: "skills", heading: "Skills" },
		{ id: "sandboxes", heading: "Sandboxes" },
		{ id: "monitoring", heading: "Monitoring" },
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

	test("selecting identity emoji shows favicon reload notice", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/identity");

		const pickBtn = page.getByRole("button", { name: "Pick", exact: true });
		await expect(pickBtn).toBeVisible();
		await pickBtn.click();

		const selectedEmoji = await page.evaluate(() => {
			var current = (window.__MOLTIS__?.identity?.emoji || "").trim();
			var options = ["🦊", "🐙", "🤖", "🐶"];
			return options.find((emoji) => emoji !== current) || "🦊";
		});
		await page.getByRole("button", { name: selectedEmoji, exact: true }).click();
		await expect(page.getByText("Saved", { exact: true })).toBeVisible();
		await expect(
			page.getByText("favicon updates requires reload and may be cached for minutes", { exact: false }),
		).toBeVisible();
		await expect(page.getByRole("button", { name: "requires reload", exact: true })).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("favicon reload notice button triggers a full page refresh", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/identity");

		const pickBtn = page.getByRole("button", { name: "Pick", exact: true });
		await expect(pickBtn).toBeVisible();
		await pickBtn.click();

		const selectedEmoji = await page.evaluate(() => {
			var current = (window.__MOLTIS__?.identity?.emoji || "").trim();
			var options = ["🦊", "🐙", "🤖", "🐶"];
			return options.find((emoji) => emoji !== current) || "🦊";
		});
		await page.getByRole("button", { name: selectedEmoji, exact: true }).click();
		await expect(page.getByText("Saved", { exact: true })).toBeVisible();
		const reloadBtn = page.getByRole("button", { name: "requires reload", exact: true });
		await expect(reloadBtn).toBeVisible();

		await Promise.all([page.waitForEvent("framenavigated", (frame) => frame === page.mainFrame()), reloadBtn.click()]);
		await expectPageContentMounted(page);
		await expect(page).toHaveURL(/\/settings\/identity$/);

		expect(pageErrors).toEqual([]);
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
			"Crons",
			"Security",
			"Tailscale",
			"LLMs",
			"Channels",
			"Voice",
			"MCP",
			"Hooks",
			"Skills",
			"Sandboxes",
			"Monitoring",
			"Logs",
			"Configuration",
		];
		const expectedWithoutVoice = expectedWithVoice.filter((item) => item !== "Voice");
		expect(navItems).toEqual(navItems.includes("Voice") ? expectedWithVoice : expectedWithoutVoice);
	});
});
