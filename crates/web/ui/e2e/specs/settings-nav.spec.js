const { expect, test } = require("../base-test");
const { navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

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
	async function openProvidersPage(page) {
		await navigateAndWait(page, "/settings/providers");
		await expect.poll(() => new URL(page.url()).pathname).toBe("/settings/providers");
		await expect(page.locator("#providersTitle")).toBeVisible();
	}

	test("/settings redirects to /settings/profile", async ({ page }) => {
		await navigateAndWait(page, "/settings");
		await expect(page).toHaveURL(/\/settings\/profile$/);
		await expect(page.getByRole("heading", { name: "User Profile", exact: true })).toBeVisible();
	});

	test("settings nav keeps distinct icons for nodes, remote access, network audit, and openclaw import", async ({
		page,
	}) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/profile");
		await expect(page.locator(".settings-sidebar-nav")).toBeVisible();

		const masks = await page.evaluate(() => {
			const readableRules = (sheet) => {
				try {
					return Array.from(sheet.cssRules || []);
				} catch {
					return [];
				}
			};

			const readRuleMask = (selector) => {
				for (const sheet of Array.from(document.styleSheets || [])) {
					const rule = readableRules(sheet).find(
						(candidate) => candidate.type === CSSRule.STYLE_RULE && candidate.selectorText === selector,
					);
					if (rule)
						return rule.style.getPropertyValue("-webkit-mask-image") || rule.style.getPropertyValue("mask-image") || "";
				}
				return null;
			};
			return {
				nodes: readRuleMask('.settings-nav-item[data-section="nodes"]::before'),
				ssh: readRuleMask('.settings-nav-item[data-section="ssh"]::before'),
				tools: readRuleMask('.settings-nav-item[data-section="tools"]::before'),
				remoteAccess: readRuleMask('.settings-nav-item[data-section="remote-access"]::before'),
				networkAudit: readRuleMask('.settings-nav-item[data-section="network-audit"]::before'),
				mcp: readRuleMask('.settings-nav-item[data-section="mcp"]::before'),
				openclawImport: readRuleMask('.settings-nav-item[data-section="import"]::before'),
			};
		});

		const hasMask = (value) => {
			if (typeof value !== "string") return false;
			const normalized = value.trim().toLowerCase();
			return normalized !== "" && normalized !== "none";
		};
		if (masks.nodes !== null) {
			expect(hasMask(masks.nodes)).toBeTruthy();
		}
		expect(hasMask(masks.ssh)).toBeTruthy();
		expect(hasMask(masks.tools)).toBeTruthy();
		expect(hasMask(masks.remoteAccess)).toBeTruthy();
		expect(hasMask(masks.networkAudit)).toBeTruthy();
		expect(hasMask(masks.mcp)).toBeTruthy();
		expect(masks.remoteAccess).not.toBe(masks.networkAudit);

		// Import appears only when OpenClaw is detected in this run.
		if (masks.openclawImport !== null) {
			expect(hasMask(masks.openclawImport)).toBeTruthy();
			expect(masks.openclawImport).not.toBe(masks.mcp);
		}

		expect(pageErrors).toEqual([]);
	});

	const settingsSections = [
		{ id: "profile", heading: "User Profile" },
		{ id: "memory", heading: "Memory" },
		{ id: "environment", heading: "Environment" },
		{ id: "crons", heading: "Cron Jobs" },
		{ id: "voice", heading: "Voice" },
		{ id: "security", heading: "Security" },
		{ id: "ssh", heading: "SSH" },
		{ id: "remote-access", heading: "Remote Access" },
		{ id: "network-audit", heading: "Network Audit" },
		{ id: "notifications", heading: "Notifications" },
		{ id: "providers", heading: "LLMs" },
		{ id: "tools", heading: "Tools" },
		{ id: "channels", heading: "Channels" },
		{ id: "mcp", heading: "MCP" },
		{ id: "hooks", heading: "Hooks" },
		{ id: "skills", heading: "Skills" },
		{ id: "projects", heading: "Repositories" },
		{ id: "sandboxes", heading: "Sandboxes" },
		{ id: "monitoring", heading: "Monitoring" },
		{ id: "logs", heading: "Logs" },
		{ id: "config", heading: "Configuration" },
	];

	for (const section of settingsSections) {
		test(`settings/${section.id} loads without errors`, async ({ page }) => {
			const pageErrors = watchPageErrors(page);
			await navigateAndWait(page, `/settings/${section.id}`);
			await waitForWsConnected(page);

			await expect(page).toHaveURL(new RegExp(`/settings/${section.id}$`));

			// Settings sections use heading text that may differ slightly
			// from the section ID; check the page loaded content.
			const content = page.locator("#pageContent");
			await expect(content).not.toBeEmpty();

			expect(pageErrors).toEqual([]);
		});
	}

	test("voice settings saves whisper base URL without requiring an API key", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/voice");
		await waitForWsConnected(page);

		const whisperRow = page
			.locator(".provider-card")
			.filter({ has: page.getByText("OpenAI Whisper", { exact: true }) })
			.first();
		await expect(whisperRow).toBeVisible();

		await whisperRow.getByRole("button", { name: "Configure", exact: true }).click();
		let modal = page
			.locator(".modal-box")
			.filter({ has: page.getByText("OpenAI Whisper", { exact: false }) })
			.last();
		await expect(modal).toBeVisible();
		await modal.locator('input[data-field="baseUrl"]').fill("http://127.0.0.1:8001/v1");
		await modal.getByRole("button", { name: "Save", exact: true }).click();

		await expect(modal).toBeHidden();

		await whisperRow.getByRole("button", { name: "Configure", exact: true }).click();
		modal = page
			.locator(".modal-box")
			.filter({ has: page.getByText("OpenAI Whisper", { exact: false }) })
			.last();
		await expect(modal).toBeVisible();
		await expect(modal.locator('input[data-field="baseUrl"]')).toHaveValue("http://127.0.0.1:8001/v1");
		expect(pageErrors).toEqual([]);
	});

	test("voice settings can clear an existing whisper base URL", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/voice");
		await waitForWsConnected(page);

		const whisperRow = page
			.locator(".provider-card")
			.filter({ has: page.getByText("OpenAI Whisper", { exact: true }) })
			.first();
		await expect(whisperRow).toBeVisible();

		await whisperRow.getByRole("button", { name: "Configure", exact: true }).click();
		let modal = page
			.locator(".modal-box")
			.filter({ has: page.getByText("OpenAI Whisper", { exact: false }) })
			.last();
		await expect(modal).toBeVisible();
		await modal.locator('input[data-field="baseUrl"]').fill("http://127.0.0.1:8001/v1");
		await modal.getByRole("button", { name: "Save", exact: true }).click();

		await expect(modal).toBeHidden();

		await whisperRow.getByRole("button", { name: "Configure", exact: true }).click();
		modal = page
			.locator(".modal-box")
			.filter({ has: page.getByText("OpenAI Whisper", { exact: false }) })
			.last();
		await expect(modal).toBeVisible();
		await expect(modal.locator('input[data-field="baseUrl"]')).toHaveValue("http://127.0.0.1:8001/v1");
		await modal.locator('input[data-field="baseUrl"]').fill("");
		await modal.getByRole("button", { name: "Save", exact: true }).click();

		await expect(modal).toBeHidden();

		await whisperRow.getByRole("button", { name: "Configure", exact: true }).click();
		modal = page
			.locator(".modal-box")
			.filter({ has: page.getByText("OpenAI Whisper", { exact: false }) })
			.last();
		await expect(modal).toBeVisible();
		await expect(modal.locator('input[data-field="baseUrl"]')).toHaveValue("");
		expect(pageErrors).toEqual([]);
	});

	test("remote access page shows tailscale and ngrok cards", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.route("**/api/auth/status", async (route) => {
			await route.fulfill({
				status: 200,
				contentType: "application/json",
				body: JSON.stringify({
					auth_disabled: false,
					authenticated: true,
					has_api_keys: false,
					has_passkeys: false,
					has_password: true,
				}),
			});
		});
		await page.route("**/api/tailscale/status", async (route) => {
			await route.fulfill({
				status: 200,
				contentType: "application/json",
				body: JSON.stringify({
					hostname: "moltis.tail-scale.ts.net",
					installed: true,
					login_name: "team@example.com",
					mode: "serve",
					tailnet: "example.ts.net",
					tailscale_ip: "100.64.0.10",
					tailscale_up: true,
					url: "https://moltis.tail-scale.ts.net",
					version: "1.88.2",
				}),
			});
		});
		await page.route("**/api/ngrok/status", async (route) => {
			await route.fulfill({
				status: 200,
				contentType: "application/json",
				body: JSON.stringify({
					authtoken_present: true,
					authtoken_source: "config",
					domain: "team-gateway.ngrok.app",
					enabled: true,
					public_url: "https://team-gateway.ngrok.app",
				}),
			});
		});

		await navigateAndWait(page, "/settings/remote-access");

		await expect(page.getByRole("heading", { name: "Remote Access", exact: true })).toBeVisible();
		// Tailscale tab is active by default
		await expect(page.getByRole("tab", { name: /Tailscale/ })).toBeVisible();
		await expect(page.getByRole("tab", { name: /ngrok/ })).toBeVisible();
		// Switch to ngrok tab to see its content
		await page.getByRole("tab", { name: /ngrok/ }).click();
		await expect(page.getByText("https://team-gateway.ngrok.app", { exact: true })).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("identity form elements render", async ({ page }) => {
		await navigateAndWait(page, "/settings/profile");

		// Identity page should have a name input and soul/description textarea
		const content = page.locator("#pageContent");
		await expect(content).not.toBeEmpty();
	});

	test("nodes page shows remote exec status doctor", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/nodes");

		await expect(page.getByRole("heading", { name: "Remote Exec Status", exact: true })).toBeVisible();
		await expect(page.getByRole("button", { name: "SSH Settings", exact: true })).toBeVisible();
		await expect(page.getByText("Backend", { exact: true })).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("tools settings shows effective inventory and routing summary", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/tools");

		await expect(page.getByRole("heading", { name: "Tools", exact: true })).toBeVisible();
		await expect(
			page.getByText("This page shows the effective tool inventory for the active session and model.", {
				exact: false,
			}),
		).toBeVisible();
		await expect(page.getByText("Tool Calling", { exact: true })).toBeVisible();
		await expect(page.getByText("Execution Routes", { exact: true })).toBeVisible();
		await expect(page.getByText("Registered Tools", { exact: true })).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("nodes join URL uses browser location port, not gon port", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/nodes");
		await waitForWsConnected(page);

		// Override gon port to a different value to simulate a reverse proxy
		// scenario where the internal bind port differs from the browser port.
		await page.evaluate(() => {
			const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			if (!appScript) throw new Error("app.js module not found");
			const appUrl = new URL(appScript.src, window.location.origin);
			const prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			return import(`${prefix}js/gon.js`).then((gon) => {
				gon.set("port", 99999);
			});
		});

		// Re-navigate so ConnectNodeForm re-renders with the spoofed gon port.
		await navigateAndWait(page, "/settings/nodes");

		const endpointCode = page.locator("code").filter({ hasText: /^wss?:\/\// });
		await expect(endpointCode).toBeVisible();
		const wsUrl = (await endpointCode.textContent()).trim();

		// The URL must use the browser's port (location.port), NOT the spoofed
		// gon port 99999 — proving we are immune to the behind-proxy bug (#426).
		expect(wsUrl).not.toContain(":99999");
		const browserPort = new URL(page.url()).port;
		if (browserPort) {
			expect(wsUrl).toContain(`:${browserPort}/ws`);
		} else {
			// Running on a default port; the URL should have no port component.
			expect(wsUrl).toMatch(/^wss?:\/\/[^:]+\/ws$/);
		}

		expect(pageErrors).toEqual([]);
	});

	test("nodes doctor can repair and clear the active SSH host pin", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		let hostPinned = false;

		await page.route("**/api/ssh/doctor", async (route) => {
			await route.fulfill({
				status: 200,
				contentType: "application/json",
				body: JSON.stringify({
					ok: true,
					exec_host: "ssh",
					ssh_binary_available: true,
					ssh_binary_version: "OpenSSH_9.9",
					paired_node_count: 0,
					managed_key_count: 1,
					encrypted_key_count: 1,
					managed_target_count: 1,
					pinned_target_count: hostPinned ? 1 : 0,
					configured_node: null,
					legacy_target: null,
					active_route: {
						target_id: 42,
						label: "SSH: prod-box",
						target: "deploy@example.com",
						port: 2222,
						host_pinned: hostPinned,
						auth_mode: "managed",
						source: "managed",
					},
					checks: [],
				}),
			});
		});
		await page.route("**/api/ssh/host-key/scan", async (route) => {
			await route.fulfill({
				status: 200,
				contentType: "application/json",
				body: JSON.stringify({
					ok: true,
					host: "example.com",
					port: 2222,
					known_host: "|1|salt|hash ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAITestKey",
				}),
			});
		});
		await page.route("**/api/ssh/targets/42/pin", async (route) => {
			if (route.request().method() === "POST") {
				hostPinned = true;
			}
			if (route.request().method() === "DELETE") {
				hostPinned = false;
			}
			await route.fulfill({
				status: 200,
				contentType: "application/json",
				body: JSON.stringify({ ok: true, id: 42 }),
			});
		});

		await navigateAndWait(page, "/settings/nodes");

		await expect(page.getByRole("button", { name: "Pin Active Route", exact: true })).toBeVisible();
		await page.getByRole("button", { name: "Pin Active Route", exact: true }).click();
		await expect(page.getByRole("button", { name: "Refresh Active Pin", exact: true })).toBeVisible();
		await expect(page.getByRole("button", { name: "Clear Active Pin", exact: true })).toBeVisible();
		await expect(page.getByText("stored host key", { exact: false })).toBeVisible();

		await page.getByRole("button", { name: "Clear Active Pin", exact: true }).click();
		await expect(page.getByRole("button", { name: "Pin Active Route", exact: true })).toBeVisible();
		await expect(page.getByText("inheriting global known_hosts policy", { exact: false })).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("nodes doctor shows actionable hint for active SSH route failures", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.route("**/api/ssh/doctor", async (route) => {
			await route.fulfill({
				status: 200,
				contentType: "application/json",
				body: JSON.stringify({
					ok: true,
					exec_host: "ssh",
					ssh_binary_available: true,
					ssh_binary_version: "OpenSSH_9.9",
					paired_node_count: 0,
					managed_key_count: 1,
					encrypted_key_count: 1,
					managed_target_count: 1,
					pinned_target_count: 1,
					configured_node: null,
					legacy_target: null,
					active_route: {
						target_id: 42,
						label: "SSH: prod-box",
						target: "deploy@example.com",
						port: 22,
						host_pinned: true,
						auth_mode: "managed",
						source: "managed",
					},
					checks: [],
				}),
			});
		});
		await page.route("**/api/ssh/doctor/test-active", async (route) => {
			await route.fulfill({
				status: 200,
				contentType: "application/json",
				body: JSON.stringify({
					ok: false,
					reachable: false,
					stdout: "",
					stderr: "Host key verification failed.",
					exit_code: 255,
					route_label: "prod-box",
					failure_code: "host_key_verification_failed",
					failure_hint:
						"SSH host verification failed. Refresh or clear the host pin if the server was rebuilt, otherwise inspect the host before trusting it.",
				}),
			});
		});

		await navigateAndWait(page, "/settings/nodes");
		await page.getByRole("button", { name: "Test Active SSH Route", exact: true }).click();
		await expect(page.getByText("Host key verification failed.", { exact: true })).toBeVisible();
		await expect(
			page.getByText(
				"Hint: SSH host verification failed. Refresh or clear the host pin if the server was rebuilt, otherwise inspect the host before trusting it.",
				{ exact: true },
			),
		).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("identity name fields autosave on blur", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/profile");

		const userNameInput = page.getByPlaceholder("e.g. Alice");
		await expect(userNameInput).toBeVisible({ timeout: 5_000 });
		const currentVal = await userNameInput.inputValue();
		const nextUserName = currentVal === "AutoUserNameA" ? "AutoUserNameB" : "AutoUserNameA";

		await userNameInput.fill(nextUserName);
		await expect(userNameInput).toHaveValue(nextUserName);
		await userNameInput.blur();

		// Wait for the "Saved" flash (confirms autosave round-tripped).
		await expect(page.getByText("Saved")).toBeVisible({ timeout: 15_000 });

		// Reload and verify the value persisted.
		await page.reload();
		await expect(page.getByPlaceholder("e.g. Alice")).toHaveValue(nextUserName, { timeout: 10_000 });

		expect(pageErrors).toEqual([]);
	});

	// Removed: "selecting identity emoji updates favicon" and "safari favicon reload notice"
	// Emoji picker moved from IdentitySection to AgentsPage in the agents architecture
	// simplification (#898). These tests tested code that no longer exists on the profile page.

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
		await openProvidersPage(page);
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

	test("graphql toggle applies immediately", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/profile");
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
		await navigateAndWait(page, "/settings/profile");

		await expect(page.locator(".settings-group-label").nth(0)).toHaveText("General");
		await expect(page.locator(".settings-group-label").nth(1)).toHaveText("Security");
		await expect(page.locator(".settings-group-label").nth(2)).toHaveText("Integrations");
		await expect(page.locator(".settings-group-label").nth(3)).toHaveText("Systems");

		const navItems = (await page.locator(".settings-nav-item").allTextContents()).map((text) => text.trim());
		const expectedPrefix = [
			"User Profile",
			"Agents",
			"Nodes",
			"Projects",
			"Environment",
			"Memory",
			"Notifications",
			"Crons",
			"Webhooks",
			"Heartbeat",
			"Authentication",
		];
		if (navItems.includes("Encryption")) expectedPrefix.push("Encryption");
		if (navItems.includes("SSH")) expectedPrefix.push("SSH");
		expectedPrefix.push(
			"Remote Access",
			"Network Audit",
			"Sandboxes",
			"Channels",
			"Hooks",
			"LLMs",
			"Tools",
			"MCP",
			"Skills",
		);
		if (navItems.includes("Imports")) expectedPrefix.push("Imports");
		const expectedSystem = ["Terminal", "Monitoring", "Logs"];
		const expected = [...expectedPrefix];
		if (navItems.includes("OpenClaw Import")) expected.push("OpenClaw Import");
		if (navItems.includes("Voice")) expected.push("Voice");
		expected.push(...expectedSystem);
		if (navItems.includes("GraphQL")) expected.push("GraphQL");
		expected.push("Configuration");
		expect(navItems).toEqual(expected);

		await expect(page.locator('.settings-nav-item[data-section="providers"]')).toHaveText("LLMs");
		await expect(page.locator('.settings-nav-item[data-section="logs"]')).toHaveText("Logs");
		await expect(page.locator('.settings-nav-item[data-section="terminal"]')).toHaveText("Terminal");
		await expect(page.locator('.settings-nav-item[data-section="config"]')).toHaveText("Configuration");

		if (navItems.includes("GraphQL")) {
			await expect(page.locator('.settings-nav-item[data-section="graphql"]')).toHaveText("GraphQL");
		}
	});
});
