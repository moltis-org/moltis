const { expect, test } = require("@playwright/test");
const { expectPageContentMounted, navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

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

function graphqlHttpStatus(page) {
	return page.evaluate(async () => {
		const response = await fetch("/graphql", {
			method: "GET",
			redirect: "manual",
		});
		return response.status;
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
		{ id: "network-audit", heading: "Network Audit" },
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
			await navigateAndWait(page, `/settings/${section.id}`);

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
		await expect(page.getByRole("heading", { name: "Authentication" })).toBeVisible();
	});

	test("encryption page shows vault status when vault is enabled", async ({ page }) => {
		await navigateAndWait(page, "/settings/vault");
		const heading = page.getByRole("heading", { name: "Encryption" });
		const hasVault = await heading.isVisible().catch(() => false);
		if (hasVault) {
			await expect(heading).toBeVisible();
			// Should show a status badge
			const badges = page.locator(".provider-item-badge");
			await expect(badges.first()).toBeVisible();
		}
	});

	test("environment page shows encrypted badges on env vars", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/environment");
		await expect(page.getByRole("heading", { name: "Environment" })).toBeVisible();
		// If env vars exist, they should have either Encrypted or Plaintext badge
		const items = page.locator(".provider-item");
		const count = await items.count();
		if (count > 0) {
			const firstItem = items.first();
			const hasBadge = await firstItem.locator(".provider-item-badge").count();
			expect(hasBadge).toBeGreaterThan(0);
			const badgeText = await firstItem.locator(".provider-item-badge").first().textContent();
			expect(["Encrypted", "Plaintext"]).toContain(badgeText.trim());
		}
		expect(pageErrors).toEqual([]);
	});

	test("provider page renders from settings", async ({ page }) => {
		await navigateAndWait(page, "/settings/providers");
		await expect(page.getByRole("heading", { name: "LLMs" })).toBeVisible();
	});

	test("terminal page renders from settings", async ({ page }) => {
		await navigateAndWait(page, "/settings/terminal");
		await expect(page.getByRole("heading", { name: "Terminal", exact: true })).toBeVisible();
		await expect(page.locator("#terminalOutput .xterm")).toHaveCount(1);
		await expect(page.locator("#terminalInput")).toHaveCount(0);
		await expect(page.locator("#terminalSize")).toHaveCount(1);
		await expect(page.locator("#terminalSize")).toHaveText(/.+/);
		await expect(page.locator("#terminalTabs")).toHaveCount(1);
		await expect(page.locator("#terminalNewTab")).toHaveCount(1);
		await expect(page.locator("#terminalHintActions")).toHaveCount(1);
		await expect(page.locator("#terminalInstallTmux")).toHaveCount(1);
	});

	test("channels add telegram token field is treated as a password", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const addButton = page.getByRole("button", { name: "Connect Telegram", exact: true });
		await expect(addButton).toBeVisible();
		await addButton.click();

		await expect(page.getByRole("heading", { name: "Connect Telegram", exact: true })).toBeVisible();
		const tokenInput = page.getByPlaceholder("123456:ABC-DEF...");
		await expect(tokenInput).toHaveAttribute("type", "password");
		await expect(tokenInput).toHaveAttribute("autocomplete", "new-password");
		await expect(tokenInput).toHaveAttribute("name", "telegram_bot_token");
		expect(pageErrors).toEqual([]);
	});

	test("graphql toggle applies immediately", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/identity");
		await waitForWsConnected(page);

		const graphQlNavItem = page.locator(".settings-nav-item", { hasText: "GraphQL" });
		const hasGraphql = (await graphQlNavItem.count()) > 0;
		test.skip(!hasGraphql, "GraphQL feature not enabled in this build");

		await graphQlNavItem.click();
		await expect(page).toHaveURL(/\/settings\/graphql$/);

		const toggleSwitch = page.locator("#graphqlToggleSwitch");
		const toggle = page.locator("#graphqlEnabledToggle");
		await expect(toggleSwitch).toBeVisible();
		const initial = await toggle.isChecked();
		const settingsUrl = new URL(page.url());
		const httpEndpoint = `${settingsUrl.origin}/graphql`;
		const wsScheme = settingsUrl.protocol === "https:" ? "wss:" : "ws:";
		const wsEndpoint = `${wsScheme}//${settingsUrl.host}/graphql`;

		await toggleSwitch.click();
		await expect.poll(() => toggle.isChecked()).toBe(!initial);

		await expect.poll(async () => graphqlHttpStatus(page)).toBe(initial ? 503 : 200);
		if (initial) {
			await expect(page.locator('iframe[title="GraphiQL Playground"]')).toHaveCount(0);
		} else {
			await expect(page.getByText(httpEndpoint, { exact: true })).toBeVisible();
			await expect(page.getByText(wsEndpoint, { exact: true })).toBeVisible();
			await expect(page.locator('iframe[title="GraphiQL Playground"]')).toBeVisible();
		}

		await toggleSwitch.click();
		await expect.poll(() => toggle.isChecked()).toBe(initial);
		await expect.poll(async () => graphqlHttpStatus(page)).toBe(initial ? 200 : 503);
		if (initial) {
			await expect(page.getByText(httpEndpoint, { exact: true })).toBeVisible();
			await expect(page.getByText(wsEndpoint, { exact: true })).toBeVisible();
			await expect(page.locator('iframe[title="GraphiQL Playground"]')).toBeVisible();
		}

		expect(pageErrors).toEqual([]);
	});

	test("sidebar groups and order match product layout", async ({ page }) => {
		await navigateAndWait(page, "/settings/identity");

		await expect(page.locator(".settings-group-label").nth(0)).toHaveText("General");
		await expect(page.locator(".settings-group-label").nth(1)).toHaveText("Security");
		await expect(page.locator(".settings-group-label").nth(2)).toHaveText("Integrations");
		await expect(page.locator(".settings-group-label").nth(3)).toHaveText("Systems");

		const navItems = (await page.locator(".settings-nav-item").allTextContents()).map((text) => text.trim());
		const expectedPrefix = [
			"Identity",
			"Agents",
			"Environment",
			"Memory",
			"Notifications",
			"Crons",
			"Heartbeat",
			"Authentication",
		];
		if (navItems.includes("Encryption")) expectedPrefix.push("Encryption");
		expectedPrefix.push("Tailscale", "Network Audit", "Sandboxes", "Channels", "Hooks", "LLMs", "MCP", "Skills");
		const expectedSystem = ["Terminal", "Monitoring", "Logs"];
		const expected = [...expectedPrefix];
		if (navItems.includes("OpenClaw Import")) expected.push("OpenClaw Import");
		if (navItems.includes("Voice")) expected.push("Voice");
		expected.push(...expectedSystem);
		if (navItems.includes("GraphQL")) expected.push("GraphQL");
		expected.push("Configuration");
		expect(navItems).toEqual(expected);

		const llmsNavItem = page.locator(".settings-nav-item", { hasText: "LLMs" });
		await expect(llmsNavItem.locator(".icon-layers")).toHaveCount(1);
		await expect(llmsNavItem.locator(".icon-server")).toHaveCount(0);

		const logsNavItem = page.locator(".settings-nav-item", { hasText: "Logs" });
		await expect(logsNavItem.locator(".icon-document")).toHaveCount(1);

		const terminalNavItem = page.locator(".settings-nav-item", { hasText: "Terminal" });
		await expect(terminalNavItem.locator(".icon-terminal")).toHaveCount(1);

		const configNavItem = page.locator(".settings-nav-item", { hasText: "Configuration" });
		await expect(configNavItem.locator(".icon-code")).toHaveCount(1);
		await expect(configNavItem.locator(".icon-document")).toHaveCount(0);

		if (navItems.includes("GraphQL")) {
			const graphQlNavItem = page.locator(".settings-nav-item", { hasText: "GraphQL" });
			await expect(graphQlNavItem.locator(".icon-graphql")).toHaveCount(1);
		}
	});
});
