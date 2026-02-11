const { expect, test } = require("@playwright/test");
const { expectPageContentMounted, navigateAndWait, watchPageErrors } = require("../helpers");

async function spoofSafari(page) {
	await page.addInitScript(() => {
		const safariUserAgent =
			"Mozilla/5.0 (Macintosh; Intel Mac OS X 14_3_1) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.3 Safari/605.1.15";
		Object.defineProperty(Navigator.prototype, "userAgent", {
			configurable: true,
			get() {
				return safariUserAgent;
			},
		});
		Object.defineProperty(Navigator.prototype, "vendor", {
			configurable: true,
			get() {
				return "Apple Computer, Inc.";
			},
		});
	});
}

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

	test("identity name fields autosave on blur", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/identity");

		const nextValues = await page.evaluate(() => {
			var id = window.__MOLTIS__?.identity || {};
			var nextBotName = id.name === "AutoBotNameA" ? "AutoBotNameB" : "AutoBotNameA";
			var nextUserName = id.user_name === "AutoUserNameA" ? "AutoUserNameB" : "AutoUserNameA";
			return { nextBotName, nextUserName };
		});

		const botNameInput = page.getByPlaceholder("e.g. Rex");
		await botNameInput.fill(nextValues.nextBotName);
		await botNameInput.blur();
		await expect(page.getByText("Saved", { exact: true })).toBeVisible();
		await expect
			.poll(() => page.evaluate(() => (window.__MOLTIS__?.identity?.name || "").trim()))
			.toBe(nextValues.nextBotName);

		const userNameInput = page.getByPlaceholder("e.g. Alice");
		await userNameInput.fill(nextValues.nextUserName);
		await userNameInput.blur();
		await expect(page.getByText("Saved", { exact: true })).toBeVisible();
		await expect
			.poll(() => page.evaluate(() => (window.__MOLTIS__?.identity?.user_name || "").trim()))
			.toBe(nextValues.nextUserName);

		expect(pageErrors).toEqual([]);
	});

	test("selecting identity emoji updates favicon live without requiring notice in Chromium", async ({ page }) => {
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
		await expect
			.poll(() =>
				page.evaluate((value) => {
					var href = document.querySelector('link[rel="icon"]')?.href || "";
					if (!href.startsWith("data:image/svg+xml,")) return false;
					var decoded = decodeURIComponent(href.slice("data:image/svg+xml,".length));
					return decoded.includes(value);
				}, selectedEmoji),
			)
			.toBeTruthy();
		await expect(
			page.getByText("favicon updates requires reload and may be cached for minutes", { exact: false }),
		).toHaveCount(0);
		await expect(page.getByRole("button", { name: "requires reload", exact: true })).toHaveCount(0);

		expect(pageErrors).toEqual([]);
	});

	test("safari shows favicon reload notice and button triggers full page refresh", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await spoofSafari(page);
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
