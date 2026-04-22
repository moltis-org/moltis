const { expect, test } = require("../base-test");
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

function isRetryableNavigationError(error) {
	const message = error?.message || String(error || "");
	return (
		message.includes("net::ERR_ABORTED") ||
		message.includes("Execution context was destroyed") ||
		message.includes("Target page, context or browser has been closed")
	);
}

async function mockChannelsStatus(page, { channels, senders = [], allowRetryOwnership = false, label }) {
	const firstMarker = channels
		.map((channel) => String(channel.name || channel.account_id || channel.details || "").trim())
		.find(Boolean);
	let lastError = null;
	for (let attempt = 0; attempt < 3; attempt++) {
		try {
			await expect.poll(() => new URL(page.url()).pathname).toBe("/settings/channels");
			await expect(page.getByRole("heading", { name: "Channels", exact: true })).toBeVisible();
			await page.waitForFunction(() => !!document.querySelector('script[type="module"][src*="js/app.js"]'));
			await page.evaluate(
				async ({ channels, senders, allowRetryOwnership, label }) => {
					const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
					if (!appScript) throw new Error("app.js script not found");
					const appUrl = new URL(appScript.src, window.location.origin).href;
					const marker = "js/app.js";
					const markerIdx = appUrl.indexOf(marker);
					if (markerIdx < 0) throw new Error("app.js marker not found in script URL");
					const prefix = appUrl.slice(0, markerIdx);
					const state = await import(`${prefix}js/state.js`);
					const channelsPage = await import(`${prefix}js/page-channels.js`);
					const wsOpen = typeof WebSocket !== "undefined" ? WebSocket.OPEN : 1;
					window.__matrixOwnershipRetryRequest = null;
					state.setWs({
						readyState: wsOpen,
						send(raw) {
							const req = JSON.parse(raw || "{}");
							const resolver = state.pending[req.id];
							if (!resolver) return;
							if (req.method === "channels.status") {
								resolver({ ok: true, payload: { channels } });
							} else if (req.method === "channels.senders.list") {
								resolver({ ok: true, payload: { senders } });
							} else if (req.method === "channels.retry_ownership" && allowRetryOwnership) {
								window.__matrixOwnershipRetryRequest = req.params;
								resolver({ ok: true, payload: { ok: true } });
							} else {
								resolver({
									ok: false,
									error: { message: `unexpected rpc in ${label}: ${req.method}` },
								});
							}
							delete state.pending[req.id];
						},
					});
					state.setConnected(true);
					if (typeof state.refreshChannelsPage === "function") {
						state.refreshChannelsPage();
					} else {
						await channelsPage.prefetchChannels();
					}
					await new Promise((resolve) => requestAnimationFrame(() => resolve()));
					await new Promise((resolve) => requestAnimationFrame(() => resolve()));
				},
				{ channels, senders, allowRetryOwnership, label },
			);
			if (channels.length === 0) {
				await expect(page.getByText("No channels connected.", { exact: false })).toBeVisible();
			} else {
				await expect
					.poll(
						async () => {
							const cardCount = await page
								.locator(".provider-card")
								.count()
								.catch(() => 0);
							const pageText = await page
								.locator("#pageContent")
								.innerText()
								.catch(() => "");
							return cardCount >= channels.length && (!firstMarker || pageText.includes(firstMarker));
						},
						{ timeout: 10_000 },
					)
					.toBe(true);
			}
			return;
		} catch (error) {
			lastError = error;
			if (!isRetryableNavigationError(error) || attempt === 2) break;
		}
	}
	if (lastError) throw lastError;
}

test.describe("Settings navigation", () => {
	async function openProvidersPage(page) {
		await navigateAndWait(page, "/settings/providers");
		await expect.poll(() => new URL(page.url()).pathname).toBe("/settings/providers");
		await expect(page.locator("#providersTitle")).toBeVisible();
	}

	test("/settings redirects to /settings/identity", async ({ page }) => {
		await navigateAndWait(page, "/settings");
		await expect(page).toHaveURL(/\/settings\/identity$/);
		await expect(page.getByRole("heading", { name: "Identity", exact: true })).toBeVisible();
	});

	test("settings nav keeps distinct icons for nodes, remote access, network audit, and openclaw import", async ({
		page,
	}) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/identity");
		await expect(page.locator(".settings-sidebar-nav")).toBeVisible();

		const masks = await page.evaluate(() => {
			const readRuleMask = (selector) => {
				for (const sheet of Array.from(document.styleSheets || [])) {
					let rules;
					try {
						rules = sheet.cssRules;
					} catch {
						continue;
					}
					if (!rules) continue;
					for (const rule of Array.from(rules)) {
						if (rule.type !== CSSRule.STYLE_RULE || rule.selectorText !== selector) continue;
						return rule.style.getPropertyValue("-webkit-mask-image") || rule.style.getPropertyValue("mask-image") || "";
					}
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
		{ id: "identity", heading: "Identity" },
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

		await page.evaluate(async () => {
			const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			if (!appScript) throw new Error("app.js script not found");
			const appUrl = new URL(appScript.src, window.location.origin).href;
			const marker = "js/app.js";
			const markerIdx = appUrl.indexOf(marker);
			if (markerIdx < 0) throw new Error("app.js marker not found in script URL");
			const prefix = appUrl.slice(0, markerIdx);
			const state = await import(`${prefix}js/state.js`);
			const wsOpen = typeof WebSocket !== "undefined" ? WebSocket.OPEN : 1;
			window.__voiceSettingsSaveSettingsRequest = null;
			state.setConnected(true);
			state.setWs({
				readyState: wsOpen,
				send(raw) {
					const req = JSON.parse(raw || "{}");
					const resolver = state.pending[req.id];
					if (!resolver) return;
					if (req.method === "voice.config.save_settings") {
						window.__voiceSettingsSaveSettingsRequest = req.params || null;
						resolver({ ok: true, payload: { ok: true } });
					} else if (req.method === "voice.providers.all") {
						resolver({
							ok: true,
							payload: {
								stt: [
									{
										id: "whisper",
										name: "OpenAI Whisper",
										type: "stt",
										category: "cloud",
										description: "Best accuracy, handles accents and background noise",
										available: true,
										enabled: false,
										keySource: "config",
										settings: { baseUrl: "http://127.0.0.1:8001/v1" },
										capabilities: { baseUrl: true },
									},
								],
								tts: [],
							},
						});
					} else {
						resolver({
							ok: false,
							error: { message: `unexpected rpc in voice settings test: ${req.method}` },
						});
					}
					delete state.pending[req.id];
				},
			});
		});

		await whisperRow.getByRole("button", { name: "Configure", exact: true }).click();
		const modal = page
			.locator(".modal-box")
			.filter({ has: page.getByText("OpenAI Whisper", { exact: false }) })
			.last();
		await expect(modal).toBeVisible();
		await modal.locator('input[data-field="baseUrl"]').fill("http://127.0.0.1:8001/v1");
		await modal.getByRole("button", { name: "Save", exact: true }).click();

		await expect.poll(() => page.evaluate(() => window.__voiceSettingsSaveSettingsRequest)).not.toBeNull();

		const sentRequest = await page.evaluate(() => window.__voiceSettingsSaveSettingsRequest);
		expect(sentRequest).toMatchObject({
			provider: "whisper",
			baseUrl: "http://127.0.0.1:8001/v1",
		});
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

		await page.evaluate(async () => {
			const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			if (!appScript) throw new Error("app.js script not found");
			const appUrl = new URL(appScript.src, window.location.origin).href;
			const marker = "js/app.js";
			const markerIdx = appUrl.indexOf(marker);
			if (markerIdx < 0) throw new Error("app.js marker not found in script URL");
			const prefix = appUrl.slice(0, markerIdx);
			const state = await import(`${prefix}js/state.js`);
			const wsOpen = typeof WebSocket !== "undefined" ? WebSocket.OPEN : 1;
			window.__voiceSettingsCurrentBaseUrl = null;
			window.__voiceSettingsRequests = [];
			window.__voiceSettingsClearBaseUrlRequest = null;
			window.__voiceSettingsProvidersAllRequests = 0;
			state.setConnected(true);
			state.setWs({
				readyState: wsOpen,
				send(raw) {
					const req = JSON.parse(raw || "{}");
					const resolver = state.pending[req.id];
					if (!resolver) return;
					if (req.method === "voice.config.save_settings") {
						window.__voiceSettingsRequests.push(req.params || null);
						window.__voiceSettingsCurrentBaseUrl =
							typeof req.params?.baseUrl === "string" ? req.params.baseUrl : window.__voiceSettingsCurrentBaseUrl;
						if (req.params?.baseUrl === "") {
							window.__voiceSettingsClearBaseUrlRequest = req.params || null;
						}
						resolver({ ok: true, payload: { ok: true } });
					} else if (req.method === "voice.providers.all") {
						window.__voiceSettingsProvidersAllRequests += 1;
						resolver({
							ok: true,
							payload: {
								stt: [
									{
										id: "whisper",
										name: "OpenAI Whisper",
										type: "stt",
										category: "cloud",
										description: "Best accuracy, handles accents and background noise",
										available: true,
										enabled: false,
										keySource: "config",
										settings: { baseUrl: window.__voiceSettingsCurrentBaseUrl || "" },
										capabilities: { baseUrl: true },
									},
								],
								tts: [],
							},
						});
					} else {
						resolver({
							ok: false,
							error: { message: `unexpected rpc in voice settings clear-base-url test: ${req.method}` },
						});
					}
					delete state.pending[req.id];
				},
			});
		});

		await whisperRow.getByRole("button", { name: "Configure", exact: true }).click();
		let modal = page
			.locator(".modal-box")
			.filter({ has: page.getByText("OpenAI Whisper", { exact: false }) })
			.last();
		await expect(modal).toBeVisible();
		await modal.locator('input[data-field="baseUrl"]').fill("http://127.0.0.1:8001/v1");
		await modal.getByRole("button", { name: "Save", exact: true }).click();

		await expect.poll(() => page.evaluate(() => window.__voiceSettingsRequests.length)).toBeGreaterThan(0);
		await expect.poll(() => page.evaluate(() => window.__voiceSettingsProvidersAllRequests)).toBeGreaterThan(0);

		await whisperRow.getByRole("button", { name: "Configure", exact: true }).click();
		modal = page
			.locator(".modal-box")
			.filter({ has: page.getByText("OpenAI Whisper", { exact: false }) })
			.last();
		await expect(modal).toBeVisible();
		await expect(modal.locator('input[data-field="baseUrl"]')).toHaveValue("http://127.0.0.1:8001/v1");
		await modal.locator('input[data-field="baseUrl"]').fill("");
		await modal.getByRole("button", { name: "Save", exact: true }).click();

		await expect.poll(() => page.evaluate(() => window.__voiceSettingsClearBaseUrlRequest)).not.toBeNull();

		const sentRequest = await page.evaluate(() => window.__voiceSettingsClearBaseUrlRequest);
		expect(sentRequest).toMatchObject({
			provider: "whisper",
			baseUrl: "",
		});
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
		await navigateAndWait(page, "/settings/identity");

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
		await navigateAndWait(page, "/settings/identity");

		// Wait for the identity form to be fully initialised (no pending save).
		await expect(page.locator('button[type="submit"]')).not.toHaveText(/Saving/, { timeout: 5_000 });

		const nextValues = await page.evaluate(() => {
			var id = window.__MOLTIS__?.identity || {};
			var nextBotName = id.name === "AutoBotNameA" ? "AutoBotNameB" : "AutoBotNameA";
			var nextUserName = id.user_name === "AutoUserNameA" ? "AutoUserNameB" : "AutoUserNameA";
			return { nextBotName, nextUserName };
		});

		const botNameInput = page.getByPlaceholder("e.g. Rex");
		await botNameInput.fill(nextValues.nextBotName);
		// Ensure fill() has propagated before triggering blur
		await expect(botNameInput).toHaveValue(nextValues.nextBotName);
		await botNameInput.blur();
		// The autosave RPC can be slow under CI load; wait for the
		// data update (authoritative) rather than the transient "Saved"
		// flash which only lasts 2 s and can be missed.
		await expect
			.poll(() => page.evaluate(() => (window.__MOLTIS__?.identity?.name || "").trim()), { timeout: 15_000 })
			.toBe(nextValues.nextBotName);

		// Wait for the first save to fully settle (nameSaving → false)
		// before triggering the second blur-save.
		await expect(page.locator('button[type="submit"]')).not.toHaveText(/Saving/, { timeout: 5_000 });

		const userNameInput = page.getByPlaceholder("e.g. Alice");
		await userNameInput.fill(nextValues.nextUserName);
		await expect(userNameInput).toHaveValue(nextValues.nextUserName);
		await userNameInput.blur();
		await expect
			.poll(() => page.evaluate(() => (window.__MOLTIS__?.identity?.user_name || "").trim()), { timeout: 15_000 })
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
		const iconHrefBefore = await page.evaluate(() => document.querySelector('link[rel="icon"]')?.href || "");
		await page.getByRole("button", { name: selectedEmoji, exact: true }).click();
		await expect(page.getByText("Saved", { exact: true })).toBeVisible();
		await expect
			.poll(() =>
				page.evaluate((beforeHref) => {
					var href = document.querySelector('link[rel="icon"]')?.href || "";
					return href.startsWith("data:image/png") && href !== beforeHref;
				}, iconHrefBefore),
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

	test("channels add matrix supports access token auth and auto-generates an account id", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);
		await expect(page.getByText(/stored in Moltis's internal database \(.+moltis\.db\)/)).toBeVisible();

		const addButton = page.getByRole("button", { name: "Connect Matrix", exact: true });
		await expect(addButton).toBeVisible();
		await addButton.click();

		const modal = page.locator(".modal-box");
		await expect(modal.getByRole("heading", { name: "Connect Matrix", exact: true })).toBeVisible();
		await expect(modal.locator('input[data-field="accountId"]')).toHaveCount(0);
		await expect(modal.locator('input[data-field="homeserver"]')).toHaveValue("https://matrix.org");
		await expect(modal.locator('input[data-field="homeserver"]')).toHaveAttribute("placeholder", "https://matrix.org");
		await expect(modal.locator('select[data-field="authMode"]')).toHaveValue("oidc");
		await expect(
			modal.getByText("Encrypted Matrix chats require OIDC or Password auth.", { exact: false }),
		).toBeVisible();
		await expect(
			modal.getByText("Use OIDC (recommended) or Password so Moltis creates and persists its own Matrix device keys", {
				exact: false,
			}),
		).toBeVisible();
		await expect(modal.getByText("verify yes", { exact: false })).toBeVisible();

		await page.evaluate(async () => {
			const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			if (!appScript) throw new Error("app.js script not found");
			const appUrl = new URL(appScript.src, window.location.origin).href;
			const marker = "js/app.js";
			const markerIdx = appUrl.indexOf(marker);
			if (markerIdx < 0) throw new Error("app.js marker not found in script URL");
			const prefix = appUrl.slice(0, markerIdx);
			const state = await import(`${prefix}js/state.js`);
			const wsOpen = typeof WebSocket !== "undefined" ? WebSocket.OPEN : 1;
			window.__matrixSettingsAddRequest = null;
			state.setConnected(true);
			state.setWs({
				readyState: wsOpen,
				send(raw) {
					const req = JSON.parse(raw || "{}");
					const resolver = state.pending[req.id];
					if (!resolver) return;
					if (req.method === "channels.add") {
						window.__matrixSettingsAddRequest = req.params || null;
						resolver({ ok: true, payload: {} });
					} else if (req.method === "channels.status") {
						resolver({ ok: true, payload: { channels: [] } });
					} else {
						resolver({ ok: false, error: { message: `unexpected rpc in matrix settings test: ${req.method}` } });
					}
					delete state.pending[req.id];
				},
			});
		});

		await modal.locator('input[data-field="homeserver"]').fill("https://matrix.example.com");
		await modal.locator('select[data-field="authMode"]').selectOption("access_token");
		await expect(
			modal.getByText("Settings -> Help & About -> Advanced -> Access Token", { exact: false }),
		).toBeVisible();
		await expect(modal.getByText("Access token auth always stays user-managed", { exact: false })).toBeVisible();
		await expect(
			modal.getByText("do not transfer that device's private encryption keys into Moltis", { exact: false }),
		).toBeVisible();
		await expect(modal.getByRole("link", { name: "Matrix setup docs", exact: true })).toHaveAttribute(
			"href",
			"https://docs.moltis.org/matrix.html",
		);
		await modal.locator('input[data-field="credential"]').fill("syt_test_token");
		await modal.getByText("Advanced Config JSON", { exact: true }).click();
		await page
			.locator('textarea[data-field="advancedConfigPatch"]')
			.fill('{"reply_to_message":true,"stream_mode":"off"}');
		await page.evaluate(() => {
			const submitButton = Array.from(document.querySelectorAll(".modal-box button.provider-btn")).find(
				(button) => button.textContent?.trim() === "Connect Matrix",
			);
			if (!(submitButton instanceof HTMLButtonElement)) {
				throw new Error("visible Matrix submit button not found");
			}
			submitButton.scrollIntoView({ block: "nearest" });
			submitButton.click();
		});

		await expect.poll(() => page.evaluate(() => window.__matrixSettingsAddRequest)).not.toBeNull();

		const sentRequest = await page.evaluate(() => window.__matrixSettingsAddRequest);
		expect(sentRequest.account_id).toMatch(/^matrix-example-com-[a-z0-9]{6}$/);
		expect(sentRequest.config).toMatchObject({
			homeserver: "https://matrix.example.com",
			access_token: "syt_test_token",
			ownership_mode: "user_managed",
			auto_join: "always",
			otp_self_approval: true,
			otp_cooldown_secs: 300,
			reply_to_message: true,
			stream_mode: "off",
		});
		expect(sentRequest.config).not.toHaveProperty("user_id");
		expect(pageErrors).toEqual([]);
	});

	test("channels add matrix supports password auth and invite policy", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const addButton = page.getByRole("button", { name: "Connect Matrix", exact: true });
		await expect(addButton).toBeVisible();
		await addButton.click();

		const modal = page.locator(".modal-box");
		await expect(modal.getByRole("heading", { name: "Connect Matrix", exact: true })).toBeVisible();
		await expect(modal.locator('select[data-field="authMode"]')).toHaveValue("oidc");

		await page.evaluate(async () => {
			const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			if (!appScript) throw new Error("app.js script not found");
			const appUrl = new URL(appScript.src, window.location.origin).href;
			const marker = "js/app.js";
			const markerIdx = appUrl.indexOf(marker);
			if (markerIdx < 0) throw new Error("app.js marker not found in script URL");
			const prefix = appUrl.slice(0, markerIdx);
			const state = await import(`${prefix}js/state.js`);
			const wsOpen = typeof WebSocket !== "undefined" ? WebSocket.OPEN : 1;
			window.__matrixSettingsAddRequest = null;
			state.setConnected(true);
			state.setWs({
				readyState: wsOpen,
				send(raw) {
					const req = JSON.parse(raw || "{}");
					const resolver = state.pending[req.id];
					if (!resolver) return;
					if (req.method === "channels.add") {
						window.__matrixSettingsAddRequest = req.params || null;
						resolver({ ok: true, payload: {} });
					} else if (req.method === "channels.status") {
						resolver({ ok: true, payload: { channels: [] } });
					} else {
						resolver({ ok: false, error: { message: `unexpected rpc in matrix settings test: ${req.method}` } });
					}
					delete state.pending[req.id];
				},
			});
		});

		await modal.locator('input[data-field="homeserver"]').fill("https://matrix.example.com");
		await modal.locator('select[data-field="authMode"]').selectOption("password");
		await expect(modal.getByText("Required for encrypted Matrix chats.", { exact: false })).toBeVisible();
		await expect(modal.getByLabel("Let Moltis own this Matrix account", { exact: true })).toBeChecked();
		await modal.locator('input[data-field="userId"]').fill("@bot:example.com");
		await modal.locator('input[data-field="credential"]').fill("correct horse battery staple");
		await modal.locator('select[data-field="autoJoin"]').selectOption("allowlist");
		const matrixDmAllowlistInput = modal
			.getByText("DM Allowlist (Matrix user IDs)", { exact: true })
			.locator("xpath=following-sibling::div[1]//input");
		const matrixRoomAllowlistInput = modal
			.getByText("Room Allowlist (room IDs or aliases)", { exact: true })
			.locator("xpath=following-sibling::div[1]//input");
		await matrixDmAllowlistInput.fill("@alice:example.com");
		await matrixDmAllowlistInput.press("Enter");
		await matrixRoomAllowlistInput.fill("@ops:example.com");
		await matrixRoomAllowlistInput.press("Enter");
		await page.evaluate(() => {
			const submitButton = Array.from(document.querySelectorAll(".modal-box button.provider-btn")).find(
				(button) => button.textContent?.trim() === "Connect Matrix",
			);
			if (!(submitButton instanceof HTMLButtonElement)) {
				throw new Error("visible Matrix submit button not found");
			}
			submitButton.scrollIntoView({ block: "nearest" });
			submitButton.click();
		});

		await expect.poll(() => page.evaluate(() => window.__matrixSettingsAddRequest)).not.toBeNull();

		const sentRequest = await page.evaluate(() => window.__matrixSettingsAddRequest);
		expect(sentRequest.account_id).toBe("bot-example-com");
		expect(sentRequest.config).toMatchObject({
			homeserver: "https://matrix.example.com",
			user_id: "@bot:example.com",
			password: "correct horse battery staple",
			ownership_mode: "moltis_owned",
			auto_join: "allowlist",
			otp_self_approval: true,
			otp_cooldown_secs: 300,
			user_allowlist: ["@alice:example.com"],
			room_allowlist: ["@ops:example.com"],
		});
		expect(sentRequest.config).not.toHaveProperty("access_token");
		expect(pageErrors).toEqual([]);
	});

	test("channels page shows matrix verification state and pending verification guidance", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");

		await mockChannelsStatus(page, {
			label: "matrix status test",
			channels: [
				{
					type: "matrix",
					account_id: "moltis-testbot",
					name: "Matrix (moltis-testbot)",
					status: "connected",
					details: "@moltis-testbot:matrix.org on https://matrix.org",
					sessions: [],
					extra: {
						matrix: {
							verification_state: "unverified",
							ownership_mode: "moltis_owned",
							auth_mode: "password",
							user_id: "@moltis-testbot:matrix.org",
							device_id: "MOLTISBOT",
							device_display_name: "Moltis Matrix Bot",
							cross_signing_complete: true,
							device_verified_by_owner: false,
							recovery_state: "enabled",
							pending_verifications: [
								{
									flow_id: "flow-1",
									other_user_id: "@alice:matrix.org",
									room_id: "!room:matrix.org",
									emoji_lines: ["🐶 Dog", "🔥 Fire"],
								},
							],
						},
					},
				},
			],
		});

		await expect(page.getByText("Matrix (moltis-testbot)", { exact: true })).toBeVisible();
		await expect(page.getByText("Encryption device state: unverified", { exact: false })).toBeVisible();
		await expect(page.getByText("Managed by Moltis", { exact: true })).toBeVisible();
		await expect(page.getByText("Device not yet verified by owner", { exact: true })).toBeVisible();
		await expect(page.getByText("MOLTISBOT", { exact: true })).toBeHidden();
		const matrixDetails = page.getByText("Matrix account details", { exact: true });
		await expect(matrixDetails).toBeVisible();
		await matrixDetails.click();
		await expect(page.getByText("@moltis-testbot:matrix.org", { exact: true })).toBeVisible();
		await expect(page.getByText("MOLTISBOT", { exact: true })).toBeVisible();
		await expect(page.getByText("Verification pending", { exact: true })).toBeVisible();
		await expect(page.getByText("With @alice:matrix.org", { exact: true })).toBeVisible();
		await expect(page.getByText("verify yes", { exact: false })).toBeVisible();
		await expect(page.getByText("verify show", { exact: false })).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("channels page shows blocked Matrix ownership state for incomplete secret storage", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");

		await mockChannelsStatus(page, {
			label: "blocked matrix ownership test",
			channels: [
				{
					type: "matrix",
					account_id: "moltis-testbot",
					name: "Matrix (moltis-testbot)",
					status: "connected",
					details: "@moltis-testbot:matrix.org on https://matrix.org",
					sessions: [],
					extra: {
						matrix: {
							verification_state: "unverified",
							ownership_mode: "moltis_owned",
							auth_mode: "password",
							user_id: "@moltis-testbot:matrix.org",
							device_id: "MOLTISBOT",
							cross_signing_complete: false,
							device_verified_by_owner: false,
							recovery_state: "incomplete",
							ownership_error:
								"invalid channel input: matrix account already has incomplete secret storage that this password could not unlock; repair the account in Element or switch to user-managed mode",
							pending_verifications: [],
						},
					},
				},
			],
		});

		await expect(page.getByText("Matrix (moltis-testbot)", { exact: true })).toBeVisible();
		await expect(page.getByText("Moltis ownership blocked", { exact: true })).toBeVisible();
		await expect(
			page.getByText(
				"This account already has partial Matrix secure-backup state. Finish or repair it in Element, or switch this channel to user-managed mode.",
				{ exact: true },
			),
		).toBeVisible();
		await expect(page.getByText("Ownership setup needs attention", { exact: true })).toBeVisible();
		await expect(
			page.getByText("matrix account already has incomplete secret storage that this password could not unlock", {
				exact: false,
			}),
		).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("channels page shows Matrix ownership approval guidance for existing accounts", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");

		await mockChannelsStatus(page, {
			label: "matrix ownership approval test",
			allowRetryOwnership: true,
			channels: [
				{
					type: "matrix",
					account_id: "moltis-testbot",
					name: "Matrix (moltis-testbot)",
					status: "connected",
					details: "@moltis-testbot:matrix.org on https://matrix.org",
					sessions: [],
					extra: {
						matrix: {
							verification_state: "unverified",
							ownership_mode: "moltis_owned",
							auth_mode: "password",
							user_id: "@moltis-testbot:matrix.org",
							device_id: "GT7YDd8CWl",
							cross_signing_complete: false,
							device_verified_by_owner: false,
							recovery_state: "disabled",
							ownership_error:
								"invalid channel input: matrix account requires browser approval to reset cross-signing at https://account.matrix.org/account/?action=org.matrix.cross_signing_reset; complete that in Element or switch to user-managed mode",
							pending_verifications: [],
						},
					},
				},
			],
		});

		await expect(page.getByText("Ownership approval required", { exact: true })).toBeVisible();
		await expect(
			page.getByText(
				"This existing Matrix account can already chat, but Matrix needs one browser approval before Moltis can take over encryption ownership. Open the approval page, approve the reset, then retry ownership setup.",
				{ exact: true },
			),
		).toBeVisible();
		await expect(page.getByText("Browser approval pending", { exact: true })).toBeVisible();
		const approvalLink = page.getByRole("link", {
			name: "Open approval page for @moltis-testbot:matrix.org",
			exact: true,
		});
		await expect(approvalLink).toHaveAttribute(
			"href",
			"https://account.matrix.org/account/?action=org.matrix.cross_signing_reset",
		);
		await expect(approvalLink).toHaveClass(/provider-btn/);
		await expect(approvalLink).not.toHaveClass(/provider-btn-secondary/);
		const retryButton = page.getByRole("button", {
			name: "Click here once you reset the account",
			exact: true,
		});
		await expect(retryButton).toBeVisible();
		const approvalNote = approvalLink.locator("xpath=../following-sibling::div[1]");
		await expect(approvalNote).toContainText("Make sure the browser page is signed into @moltis-testbot:matrix.org.");
		await retryButton.click();
		await expect.poll(() => page.evaluate(() => window.__matrixOwnershipRetryRequest)).not.toBeNull();
		const retryRequest = await page.evaluate(() => window.__matrixOwnershipRetryRequest);
		expect(retryRequest).toEqual({
			type: "matrix",
			account_id: "moltis-testbot",
		});
		expect(pageErrors).toEqual([]);
	});

	test("senders page shows pending matrix sender with one visible sigil and OTP code", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");

		await page.evaluate(async () => {
			const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			if (!appScript) throw new Error("app.js script not found");
			const appUrl = new URL(appScript.src, window.location.origin).href;
			const marker = "js/app.js";
			const markerIdx = appUrl.indexOf(marker);
			if (markerIdx < 0) throw new Error("app.js marker not found in script URL");
			const prefix = appUrl.slice(0, markerIdx);
			const state = await import(`${prefix}js/state.js`);
			const channelsPage = await import(`${prefix}js/page-channels.js`);
			const wsOpen = typeof WebSocket !== "undefined" ? WebSocket.OPEN : 1;
			state.setWs({
				readyState: wsOpen,
				send(raw) {
					const req = JSON.parse(raw || "{}");
					const resolver = state.pending[req.id];
					if (!resolver) return;
					if (req.method === "channels.status") {
						resolver({
							ok: true,
							payload: {
								channels: [
									{
										type: "matrix",
										account_id: "moltis-testbot",
										name: "Matrix (moltis-testbot)",
										status: "connected",
										details: "@moltis-testbot:matrix.org on https://matrix.org",
										sessions: [],
									},
								],
							},
						});
					} else if (req.method === "channels.senders.list") {
						resolver({
							ok: true,
							payload: {
								senders: [
									{
										peer_id: "@alice:matrix.org",
										username: "@alice:matrix.org",
										sender_name: "Alice",
										message_count: 1,
										last_seen: 1700000000,
										allowed: false,
										otp_pending: {
											code: "954502",
											expires_at: 1700000300,
										},
									},
								],
							},
						});
					} else {
						resolver({ ok: false, error: { message: `unexpected rpc in matrix senders test: ${req.method}` } });
					}
					delete state.pending[req.id];
				},
			});
			state.setConnected(true);
			await channelsPage.prefetchChannels();
			await new Promise((resolve) => requestAnimationFrame(() => resolve()));
		});

		await expect(page.getByText("Matrix (moltis-testbot)", { exact: true })).toBeVisible();
		await page.getByRole("tab", { name: /Senders/ }).click();
		await expect.poll(() => page.locator(".senders-table tbody tr").count(), { timeout: 10_000 }).toBe(1);
		await expect(page.getByText("Alice", { exact: true })).toBeVisible();
		await expect(page.getByText("@alice:matrix.org", { exact: true })).toBeVisible();
		await expect(page.getByText("@@alice:matrix.org", { exact: true })).toHaveCount(0);
		await expect(page.getByText("954502", { exact: true })).toBeVisible();
		await expect(page.getByText("Approve", { exact: true })).toBeVisible();
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
