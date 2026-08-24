const { expect, test } = require("../base-test");
const {
	createSession,
	expectPageContentMounted,
	navigateAndWait,
	sendRpcFromPage,
	waitForWsConnected,
	watchPageErrors,
} = require("../helpers");

async function waitForWelcomeOrNoProvidersCard(page) {
	await page.waitForSelector("#welcomeCard, #noProvidersCard", {
		state: "visible",
		timeout: 10_000,
	});

	// The two cards can swap during load: if models haven't arrived yet when the
	// session opens, #noProvidersCard is rendered first and then replaced with
	// #welcomeCard once models load (see refreshWelcomeCardIfNeeded in
	// sessions.js). Prefer the welcome card if it eventually appears, and only
	// treat the no-providers state as final when the welcome card never shows.
	const welcomeCard = page.locator("#welcomeCard");
	try {
		await expect(welcomeCard).toBeVisible({ timeout: 5_000 });
		return welcomeCard;
	} catch {
		// Welcome card never materialized — we're in the no-providers state.
	}

	const noProvidersCard = page.locator("#noProvidersCard");
	await expect(noProvidersCard).toBeVisible();
	await expect(noProvidersCard.getByRole("heading", { name: "No LLMs Connected", exact: true })).toBeVisible();
	await expect(noProvidersCard.getByRole("link", { name: "Go to LLMs", exact: true })).toBeVisible();
	return null;
}

async function deleteAgentByName(page, agentName) {
	await navigateAndWait(page, "/settings/agents");
	const testCard = page
		.locator(".backend-card")
		.filter({ hasText: agentName })
		.filter({ has: page.getByRole("button", { name: "Delete", exact: true }) });
	await expect(testCard).toBeVisible({ timeout: 10_000 });
	await testCard.getByRole("button", { name: "Delete", exact: true }).click();
	await page.locator(".provider-modal").getByRole("button", { name: "Delete", exact: true }).click();
	await expect(testCard).toHaveCount(0, { timeout: 10_000 });
}

async function mockExternalAgentsRpc(page, listPayload, modelsPayload, bindFailures = 0, holdBackendSwitches = false) {
	if (Array.isArray(modelsPayload)) {
		await page.route(
			"**/api/bootstrap?**",
			async (route) => {
				await route.fulfill({
					status: 200,
					contentType: "application/json",
					body: JSON.stringify({ models: modelsPayload }),
				});
			},
			{ times: 1 },
		);
	}
	await page.route(/\/api\/sessions(?:\?.*)?$/, async (route) => {
		const response = await route.fetch();
		const payload = await response.json();
		const bindings = await page.evaluate(() => window.__externalAgentE2EBindings || {});
		const sessions = Array.isArray(payload) ? payload : payload.sessions;
		if (Array.isArray(sessions)) {
			for (const session of sessions) {
				if (Object.hasOwn(bindings, session.key)) session.external_agent_kind = bindings[session.key];
			}
		}
		await route.fulfill({ response, json: payload });
	});
	await page.addInitScript(
		({ externalAgentsListPayload, modelListPayload, bindFailureCount, holdSwitches }) => {
			if (window.__externalAgentE2EPatched) return;
			window.__externalAgentE2EPatched = true;
			window.__externalAgentE2ERequests = [];
			window.__externalAgentE2EBindings = {};
			window.__externalAgentE2EPendingResponses = [];
			window.__externalAgentE2EHoldSwitches = holdSwitches;
			window.__releaseExternalAgentE2EResponses = () => {
				const pending = window.__externalAgentE2EPendingResponses.splice(0);
				for (const sendResponse of pending) sendResponse();
			};
			let failuresRemaining = bindFailureCount;
			const agentsPayload = externalAgentsListPayload || [
				{ kind: "codex", name: "Codex", installed: true, isAcp: false, version: null },
				{ kind: "claude-code", name: "Claude Code", installed: false, isAcp: false, version: null },
			];
			const originalSend = WebSocket.prototype.send;

			function respond(socket, id, payload) {
				queueMicrotask(() => {
					const event = new MessageEvent("message", {
						data: JSON.stringify({ type: "res", id, ok: true, payload }),
					});
					if (typeof socket.onmessage === "function") socket.onmessage(event);
				});
			}

			function respondError(socket, id, message) {
				queueMicrotask(() => {
					const event = new MessageEvent("message", {
						data: JSON.stringify({ type: "res", id, ok: false, error: { message } }),
					});
					if (typeof socket.onmessage === "function") socket.onmessage(event);
				});
			}

			function respondToBackendSwitch(sendResponse) {
				if (window.__externalAgentE2EHoldSwitches) {
					window.__externalAgentE2EPendingResponses.push(sendResponse);
					return;
				}
				sendResponse();
			}

			WebSocket.prototype.send = function (payload) {
				try {
					var parsed = JSON.parse(payload);
					if (parsed?.method === "models.list" && Array.isArray(modelListPayload)) {
						respond(this, parsed.id, modelListPayload);
						return;
					}
					if (parsed?.method === "external_agents.list") {
						window.__externalAgentE2ERequests.push({ method: parsed.method, params: parsed.params || {} });
						respond(this, parsed.id, agentsPayload);
						return;
					}
					if (parsed?.method === "external_agents.bind") {
						window.__externalAgentE2ERequests.push({ method: parsed.method, params: parsed.params || {} });
						if (failuresRemaining > 0) {
							failuresRemaining--;
							respondError(this, parsed.id, "simulated bind failure");
							return;
						}
						respondToBackendSwitch(() => {
							window.__externalAgentE2EBindings[parsed.params?.sessionKey] = parsed.params?.kind;
							respond(this, parsed.id, { ok: true });
						});
						return;
					}
					if (parsed?.method === "external_agents.unbind") {
						window.__externalAgentE2ERequests.push({ method: parsed.method, params: parsed.params || {} });
						respondToBackendSwitch(() => {
							window.__externalAgentE2EBindings[parsed.params?.sessionKey] = null;
							respond(this, parsed.id, { ok: true });
						});
						return;
					}
					if (parsed?.method === "sessions.patch") {
						window.__externalAgentE2ERequests.push({ method: parsed.method, params: parsed.params || {} });
					}
				} catch (_err) {
					// Fall through to the original sender.
				}
				return originalSend.call(this, payload);
			};
		},
		{
			externalAgentsListPayload: listPayload,
			modelListPayload: modelsPayload,
			bindFailureCount: bindFailures,
			holdSwitches: holdBackendSwitches,
		},
	);
}

async function expectActiveSessionExternalAgent(page, kind) {
	await expect
		.poll(
			async () =>
				page.evaluate((nextKind) => {
					const session = window.__moltis_stores?.sessionStore?.activeSession?.value;
					if (!session) return undefined;
					session.external_agent_kind = nextKind;
					session.dataVersion.value++;
					return session.external_agent_kind;
				}, kind),
			{ timeout: 10_000 },
		)
		.toBe(kind);
}

test.describe("Agents settings page", () => {
	test("settings/agents loads and retries modes after a timeout", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.addInitScript(() => {
			window.__agentsModesListAttempts = 0;
			const originalSend = WebSocket.prototype.send;
			WebSocket.prototype.send = function (payload) {
				try {
					const parsed = JSON.parse(typeof payload === "string" ? payload : "");
					if (parsed?.method === "modes.list") {
						window.__agentsModesListAttempts++;
						if (window.__agentsModesListAttempts === 1) {
							queueMicrotask(() => {
								this.dispatchEvent(
									new MessageEvent("message", {
										data: JSON.stringify({
											type: "res",
											id: parsed.id,
											ok: false,
											error: { code: "TIMEOUT", message: "RPC request timed out (modes.list)" },
										}),
									}),
								);
							});
							return;
						}
					}
				} catch (_err) {
					// Fall through to the original sender.
				}
				return originalSend.call(this, payload);
			};
		});
		await navigateAndWait(page, "/settings/agents");

		await expect(page).toHaveURL(/\/settings\/agents$/);
		await expect(page.getByRole("heading", { name: "Agents", exact: true })).toBeVisible();
		await expect(page.getByRole("tab", { name: /Chat Agents/ })).toBeVisible();
		await expect(page.getByRole("tab", { name: /Sub-Agents/ })).toBeVisible();
		await expect(page.getByRole("tab", { name: /Modes/ })).toBeVisible();

		const chatPanel = page.getByLabel("Chat Agents panel");
		await expect(chatPanel.getByText("Persistent identities with their own memory", { exact: false })).toBeVisible();

		await page.getByRole("tab", { name: /Modes/ }).click();
		const modesPanel = page.getByLabel("Modes panel");
		await expect(modesPanel.getByText("Temporary per-session prompt overlays", { exact: false })).toBeVisible();
		await expect(modesPanel.locator(".backend-card").filter({ hasText: "Concise" })).toBeVisible({
			timeout: 10_000,
		});
		await expect(modesPanel.locator(".backend-card").filter({ hasText: "Review" })).toBeVisible();
		await expect.poll(() => page.evaluate(() => window.__agentsModesListAttempts)).toBe(2);

		expect(pageErrors).toEqual([]);
	});

	test("main agent card is shown with Default badge", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/agents");

		const mainCard = page.locator(".backend-card").filter({ hasText: "Default" });
		await expect(mainCard).toBeVisible();

		// Main agent has Edit but no Delete (cannot delete the main agent)
		await expect(mainCard.getByRole("button", { name: "Edit", exact: true })).toBeVisible();
		await expect(mainCard.getByRole("button", { name: "Delete", exact: true })).toHaveCount(0);

		expect(pageErrors).toEqual([]);
	});

	test("New Agent button opens create form", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/agents");

		const newBtn = page.getByRole("button", { name: "New Agent", exact: true });
		await expect(newBtn).toBeVisible();
		await newBtn.click();

		// Form should be visible with ID, Name, and Create/Cancel buttons
		await expect(page.getByText("Create Agent", { exact: true })).toBeVisible();
		await expect(page.getByPlaceholder("e.g. writer, coder, researcher")).toBeVisible();
		await expect(page.getByPlaceholder("Creative Writer")).toBeVisible();
		await expect(page.getByRole("button", { name: "Create", exact: true })).toBeVisible();
		await expect(page.getByRole("button", { name: "Cancel", exact: true })).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("config-only preset can be promoted into an agent", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/agents");
		await waitForWsConnected(page);
		await sendRpcFromPage(page, "agents.delete", { id: "coder" });
		await navigateAndWait(page, "/settings/agents");
		await waitForWsConnected(page);

		await page.getByRole("tab", { name: /Sub-Agents/ }).click();
		await expect(page.getByRole("heading", { name: "Sub-Agent Presets", exact: true })).toBeVisible({
			timeout: 10_000,
		});
		await expect(page.getByText("usable by spawn_agent", { exact: false })).toBeVisible();
		const presetCard = page
			.locator(".backend-card")
			.filter({ hasText: "Coder" })
			.filter({ hasText: "Built-in" })
			.first();
		await expect(presetCard).toBeVisible({ timeout: 10_000 });
		await presetCard.getByRole("button", { name: "Add to Chat", exact: true }).click();
		await expect(presetCard).toHaveCount(0, { timeout: 10_000 });
		await page.getByRole("tab", { name: /Chat Agents/ }).click();

		const agentCard = page
			.locator(".backend-card")
			.filter({ hasText: "Coder" })
			.filter({ has: page.getByRole("button", { name: "Edit", exact: true }) })
			.first();
		await expect(agentCard).toBeVisible({ timeout: 10_000 });

		try {
			await agentCard.getByRole("button", { name: "Edit", exact: true }).click();
			await expect(page.getByText("Edit Coder", { exact: true })).toBeVisible({ timeout: 10_000 });
			await expect(page.locator("textarea").first()).toHaveValue(/Implement scoped code changes/);
			await page.getByRole("button", { name: "Cancel", exact: true }).click();
		} finally {
			await deleteAgentByName(page, "Coder");
		}

		expect(pageErrors).toEqual([]);
	});

	test("sub-agent preset can be created edited and deleted", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/agents");
		await waitForWsConnected(page);
		await sendRpcFromPage(page, "agents.preset.delete", { id: "e2e-sub-agent" });

		await page.getByRole("tab", { name: /Sub-Agents/ }).click();
		await page.getByRole("button", { name: "New Sub-Agent", exact: true }).click();
		await expect(page.getByText("Create Sub-Agent", { exact: true })).toBeVisible();
		await page.getByPlaceholder("e.g. researcher, reviewer, qa-helper").fill("e2e-sub-agent");
		await page.getByPlaceholder("Research Specialist").fill("E2E Sub Agent");
		await page
			.getByPlaceholder("Give this sub-agent a focused role and constraints...")
			.fill("Answer with concise evidence.");
		await page.getByPlaceholder("Read, Glob, Grep, web_search").fill("Read, Grep");
		await page.getByRole("button", { name: "Create", exact: true }).click();

		const presetCard = page.locator(".backend-card").filter({ hasText: "E2E Sub Agent" });
		await expect(presetCard).toBeVisible({ timeout: 10_000 });
		await expect(presetCard.getByText("Custom", { exact: true })).toBeVisible();

		await presetCard.getByRole("button", { name: "Edit", exact: true }).click();
		await expect(page.getByText("Edit E2E Sub Agent", { exact: true })).toBeVisible();
		await page.getByPlaceholder("Research Specialist").fill("E2E Edited Sub Agent");
		await page.getByRole("button", { name: "Save", exact: true }).click();
		const editedCard = page.locator(".backend-card").filter({ hasText: "E2E Edited Sub Agent" });
		await expect(editedCard).toBeVisible({ timeout: 10_000 });

		await editedCard.getByRole("button", { name: "Delete", exact: true }).click();
		await page.locator(".provider-modal").getByRole("button", { name: "Delete", exact: true }).click();
		await expect(editedCard).toHaveCount(0, { timeout: 10_000 });

		expect(pageErrors).toEqual([]);
	});

	test("create form Cancel button returns to list", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/agents");

		await page.getByRole("button", { name: "New Agent", exact: true }).click();
		await expect(page.getByText("Create Agent", { exact: true })).toBeVisible();

		await page.getByRole("button", { name: "Cancel", exact: true }).click();

		// Should be back to the agent list with heading and New Agent button
		await expect(page.getByRole("heading", { name: "Agents", exact: true })).toBeVisible();
		await expect(page.getByRole("button", { name: "New Agent", exact: true })).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("create, edit, and delete an agent", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/agents");

		// Create a new agent
		await page.getByRole("button", { name: "New Agent", exact: true }).click();
		await expect(page.getByText("Create Agent", { exact: true })).toBeVisible();

		const idInput = page.getByPlaceholder("e.g. writer, coder, researcher");
		const nameInput = page.getByPlaceholder("Creative Writer");
		await idInput.fill("e2e-test-agent");
		await nameInput.fill("E2E Test Agent");
		await page.getByRole("button", { name: "Create", exact: true }).click();

		// Should return to the list and show the new agent
		await expect(page.getByRole("heading", { name: "Agents", exact: true })).toBeVisible({ timeout: 10_000 });
		const agentCard = page.locator(".backend-card").filter({ hasText: "E2E Test Agent" });
		await expect(agentCard).toBeVisible();
		await expect(agentCard.getByRole("button", { name: "Edit", exact: true })).toBeVisible();
		await expect(agentCard.getByRole("button", { name: "Delete", exact: true })).toBeVisible();

		// Edit the agent
		await agentCard.getByRole("button", { name: "Edit", exact: true }).click();
		await expect(page.getByText("Edit E2E Test Agent", { exact: true })).toBeVisible();

		const editNameInput = page.getByPlaceholder("Creative Writer");
		await editNameInput.fill("E2E Renamed Agent");
		await page.getByRole("button", { name: "Save", exact: true }).click();

		// Should return to the list with updated name
		await expect(page.getByRole("heading", { name: "Agents", exact: true })).toBeVisible({ timeout: 10_000 });
		const renamedCard = page.locator(".backend-card").filter({ hasText: "E2E Renamed Agent" });
		await expect(renamedCard).toBeVisible();

		// Delete the agent
		await renamedCard.getByRole("button", { name: "Delete", exact: true }).click();
		// confirmDialog shows a custom modal — click the modal's Delete button
		await page.locator(".provider-modal").getByRole("button", { name: "Delete", exact: true }).click();

		// Agent should be removed from the list
		await expect(renamedCard).toHaveCount(0, { timeout: 10_000 });

		expect(pageErrors).toEqual([]);
	});

	test("session header agent selector switches session agent and shows sidebar indicator", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		let agentCreated = false;
		try {
			await navigateAndWait(page, "/settings/agents");
			await waitForWsConnected(page);

			await page.getByRole("button", { name: "New Agent", exact: true }).click();
			await expect(page.getByText("Create Agent", { exact: true })).toBeVisible();
			await page.getByPlaceholder("e.g. writer, coder, researcher").fill("selector-test");
			await page.getByPlaceholder("Creative Writer").fill("Selector Test Agent");
			await page.getByRole("button", { name: "Create", exact: true }).click();
			await expect(page.locator('.backend-card[data-agent-id="selector-test"]')).toBeVisible({ timeout: 10_000 });
			agentCreated = true;

			await page.goto("/chats");
			await expectPageContentMounted(page);
			await waitForWsConnected(page);
			await createSession(page);

			const agentCombo = page.locator("#sessionHeaderToolbarMount .model-combo").first();
			await expect(agentCombo).toBeVisible({ timeout: 10_000 });
			const agentComboBtn = agentCombo.locator(".model-combo-btn");
			await expect(agentComboBtn).toBeEnabled({ timeout: 10_000 });
			await agentComboBtn.click();
			const agentDropdown = agentCombo.locator(".model-dropdown");
			await expect(agentDropdown).toBeVisible({ timeout: 10_000 });
			const selectorOption = agentDropdown.locator(".model-dropdown-item", { hasText: "Selector Test Agent" }).first();
			await expect(selectorOption).toBeVisible({ timeout: 10_000 });
			await selectorOption.click();
			await expect
				.poll(async () => page.evaluate(() => window.__moltis_stores?.sessionStore?.activeSession?.value?.agent_id), {
					timeout: 15_000,
				})
				.toBe("selector-test");
			await expect
				.poll(async () => {
					return (
						(await page
							.locator("#sessionList .session-item.active")
							.first()
							.textContent()
							.catch(() => "")) || ""
					);
				})
				.toContain("@selector-test");
		} finally {
			if (agentCreated) await deleteAgentByName(page, "Selector Test Agent");
		}

		expect(pageErrors).toEqual([]);
	});

	test("external-agent binding RPC binds and unbinds a session", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await mockExternalAgentsRpc(page);
		await page.goto("/chats");
		await expectPageContentMounted(page);
		await waitForWsConnected(page);
		await createSession(page);

		const sessionKey = await page.evaluate(() => window.__moltis_stores?.sessionStore?.activeSessionKey?.value || "");
		const bindResponse = await sendRpcFromPage(page, "external_agents.bind", { sessionKey, kind: "codex" });
		expect(bindResponse?.ok).toBe(true);
		await expect
			.poll(
				async () =>
					page.evaluate(() =>
						(window.__externalAgentE2ERequests || []).some(
							(req) => req.method === "external_agents.bind" && req.params?.kind === "codex",
						),
					),
				{ timeout: 10_000 },
			)
			.toBe(true);
		await expectActiveSessionExternalAgent(page, "codex");

		const unbindResponse = await sendRpcFromPage(page, "external_agents.unbind", { sessionKey });
		expect(unbindResponse?.ok).toBe(true);
		await expect
			.poll(
				async () =>
					page.evaluate(
						(key) =>
							(window.__externalAgentE2ERequests || []).some(
								(req) => req.method === "external_agents.unbind" && req.params?.sessionKey === key,
							),
						sessionKey,
					),
				{ timeout: 10_000 },
			)
			.toBe(true);
		await expectActiveSessionExternalAgent(page, null);

		expect(pageErrors).toEqual([]);
	});

	test("composer selector lists and binds named ACP agents", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await mockExternalAgentsRpc(
			page,
			[
				{ kind: "acp-copilot", name: "ACP: Copilot", installed: true, isAcp: true, version: null },
				{ kind: "acp-codex", name: "ACP: Codex", installed: true, isAcp: true, version: null },
				{ kind: "acp-claude", name: "ACP: Claude", installed: true, isAcp: true, version: null },
				{ kind: "acp-pi", name: "ACP: Pi", installed: true, isAcp: true, version: null },
				{ kind: "acp-opencode", name: "ACP: opencode", installed: true, isAcp: true, version: null },
				{ kind: "acp-gemini", name: "ACP: Gemini", installed: true, isAcp: true, version: null },
				{ kind: "acp-augment", name: "ACP: Augment", installed: true, isAcp: true, version: null },
				{ kind: "acp-kiro", name: "ACP: Kiro", installed: true, isAcp: true, version: null },
				{ kind: "acp-openclaw", name: "ACP: OpenClaw", installed: true, isAcp: true, version: null },
				{ kind: "acp-openhands", name: "ACP: OpenHands", installed: true, isAcp: true, version: null },
				{ kind: "acp-kimi", name: "ACP: Kimi", installed: true, isAcp: true, version: null },
				{ kind: "acp-minimax-code", name: "ACP: MiniMax Code", installed: true, isAcp: true, version: null },
				{ kind: "acp-stakpak", name: "ACP: Stakpak", installed: true, isAcp: true, version: null },
				{ kind: "acp-fast-agent", name: "ACP: fast-agent", installed: true, isAcp: true, version: null },
			],
			[{ id: "e2e/model", displayName: "E2E Model", provider: "e2e", supportsReasoning: true }],
		);
		await page.goto("/chats");
		await expectPageContentMounted(page);
		await waitForWsConnected(page);
		await createSession(page);
		const sessionKey = await page.evaluate(() => window.__moltis_stores?.sessionStore?.activeSessionKey?.value || "");
		await page.evaluate(() => window.__moltis_stores?.modelStore?.select("e2e/model"));

		await expect(page.getByTestId("external-agent-picker")).toHaveCount(0);
		const picker = page.locator("#modelComboBtn");
		await expect(picker).toBeEnabled({ timeout: 10_000 });
		await expect(page.locator("#reasoningCombo")).toBeVisible();
		await page.locator("#reasoningComboBtn").click();
		await page
			.locator("#reasoningDropdownList .model-dropdown-item")
			.filter({ hasText: /^High$/ })
			.click();
		await expect(page.locator("#reasoningComboLabel")).toHaveText("High");
		await picker.click();
		const dropdown = page.locator("#modelDropdownList");
		await expect(dropdown.getByText("E2E Model", { exact: true })).toBeVisible();
		await expect(page.getByText("ACP: Copilot", { exact: true })).toBeVisible();
		await expect(
			dropdown.locator(".model-dropdown-item", { hasText: "ACP: Copilot" }).locator(".model-item-provider"),
		).toHaveText("ACP agent");
		await expect(page.getByText("ACP: Codex", { exact: true })).toBeVisible();
		await expect(page.getByText("ACP: Claude", { exact: true })).toBeVisible();
		await expect(page.getByText("ACP: Pi", { exact: true })).toBeVisible();
		await expect(page.getByText("ACP: opencode", { exact: true })).toBeVisible();
		await expect(page.getByText("ACP: Gemini", { exact: true })).toBeVisible();
		await expect(page.getByText("ACP: Augment", { exact: true })).toBeVisible();
		await expect(page.getByText("ACP: Kiro", { exact: true })).toBeVisible();
		await expect(page.getByText("ACP: OpenClaw", { exact: true })).toBeVisible();
		await expect(page.getByText("ACP: OpenHands", { exact: true })).toBeVisible();
		await expect(page.getByText("ACP: Kimi", { exact: true })).toBeVisible();
		await expect(page.getByText("ACP: MiniMax Code", { exact: true })).toBeVisible();
		await expect(page.getByText("ACP: Stakpak", { exact: true })).toBeVisible();
		await expect(page.getByText("ACP: fast-agent", { exact: true })).toBeVisible();

		await page.getByText("ACP: Copilot", { exact: true }).click();
		await expect
			.poll(
				async () =>
					page.evaluate(
						(key) =>
							(window.__externalAgentE2ERequests || []).some(
								(req) =>
									req.method === "external_agents.bind" &&
									req.params?.sessionKey === key &&
									req.params?.kind === "acp-copilot",
							),
						sessionKey,
					),
				{ timeout: 10_000 },
			)
			.toBe(true);
		await expect(picker).toBeEnabled();
		await expect(page.locator("#modelComboLabel")).toHaveText("ACP: Copilot");
		await expect(page.locator("#reasoningCombo")).toBeHidden();

		await picker.click();
		await dropdown.getByText("E2E Model", { exact: true }).click();
		await expect
			.poll(
				async () =>
					page.evaluate(
						(key) =>
							(window.__externalAgentE2ERequests || []).some(
								(req) => req.method === "external_agents.unbind" && req.params?.sessionKey === key,
							),
						sessionKey,
					),
				{ timeout: 10_000 },
			)
			.toBe(true);
		await expect(page.locator("#modelComboLabel")).toHaveText("E2E Model");
		await expect(page.locator("#reasoningCombo")).toBeVisible();
		await expect(page.locator("#reasoningComboLabel")).toHaveText("High");
		await expect
			.poll(
				async () =>
					page.evaluate(
						(key) =>
							(window.__externalAgentE2ERequests || []).some(
								(req) =>
									req.method === "sessions.patch" && req.params?.key === key && req.params?.model === "e2e/model",
							),
						sessionKey,
					),
				{ timeout: 10_000 },
			)
			.toBe(true);

		expect(pageErrors).toEqual([]);
	});

	test("composer selector hides unavailable ACP agents", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await mockExternalAgentsRpc(page, [
			{ kind: "acp-copilot", name: "ACP: Copilot", installed: false, isAcp: true, version: null },
			{ kind: "acp-codex", name: "ACP: Codex", installed: false, isAcp: true, version: null },
			{ kind: "acp-opencode", name: "ACP: opencode", installed: false, isAcp: true, version: null },
			{ kind: "acp-gemini", name: "ACP: Gemini", installed: false, isAcp: true, version: null },
			{ kind: "acp-augment", name: "ACP: Augment", installed: false, isAcp: true, version: null },
			{ kind: "acp-kiro", name: "ACP: Kiro", installed: false, isAcp: true, version: null },
			{ kind: "acp-openclaw", name: "ACP: OpenClaw", installed: false, isAcp: true, version: null },
			{ kind: "acp-openhands", name: "ACP: OpenHands", installed: false, isAcp: true, version: null },
			{ kind: "acp-kimi", name: "ACP: Kimi", installed: false, isAcp: true, version: null },
			{ kind: "acp-minimax-code", name: "ACP: MiniMax Code", installed: false, isAcp: true, version: null },
			{ kind: "acp-stakpak", name: "ACP: Stakpak", installed: false, isAcp: true, version: null },
			{ kind: "acp-fast-agent", name: "ACP: fast-agent", installed: false, isAcp: true, version: null },
		]);
		await page.goto("/chats");
		await expectPageContentMounted(page);
		await waitForWsConnected(page);
		await createSession(page);

		await expect
			.poll(
				async () =>
					page.evaluate(() =>
						(window.__externalAgentE2ERequests || []).some((req) => req.method === "external_agents.list"),
					),
				{ timeout: 10_000 },
			)
			.toBe(true);
		await expect(page.getByTestId("external-agent-picker")).toHaveCount(0);
		await page.locator("#modelComboBtn").click();
		const dropdown = page.locator("#modelDropdownList");
		for (const name of [
			"ACP: Copilot",
			"ACP: Codex",
			"ACP: opencode",
			"ACP: Gemini",
			"ACP: Augment",
			"ACP: Kiro",
			"ACP: OpenClaw",
			"ACP: OpenHands",
			"ACP: Kimi",
			"ACP: MiniMax Code",
			"ACP: Stakpak",
			"ACP: fast-agent",
		]) {
			await expect(dropdown.getByText(name, { exact: true })).toHaveCount(0);
		}

		expect(pageErrors).toEqual([]);
	});

	test("backend switch only disables the originating session selector", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await mockExternalAgentsRpc(
			page,
			[{ kind: "acp-copilot", name: "ACP: Copilot", installed: true, isAcp: true, version: null }],
			[{ id: "e2e/model", displayName: "E2E Model", provider: "e2e", supportsReasoning: true }],
			0,
			true,
		);
		await page.goto("/chats");
		await expectPageContentMounted(page);
		await waitForWsConnected(page);

		const firstSessionKey = await page.evaluate(
			() => window.__moltis_stores?.sessionStore?.activeSessionKey?.value || "",
		);
		const picker = page.locator("#modelComboBtn");
		await expect(picker).toBeEnabled({ timeout: 10_000 });
		await picker.click();
		await page.locator("#modelDropdownList").getByText("ACP: Copilot", { exact: true }).click();
		await expect(picker).toBeDisabled();

		await createSession(page);
		const secondSessionKey = await page.evaluate(
			() => window.__moltis_stores?.sessionStore?.activeSessionKey?.value || "",
		);
		expect(secondSessionKey).not.toBe(firstSessionKey);
		await expect(picker).toBeEnabled();

		await page.evaluate(() => window.__releaseExternalAgentE2EResponses?.());
		await expect
			.poll(
				async () =>
					page.evaluate(
						(key) => window.__moltis_stores?.sessionStore?.getByKey?.(key)?.external_agent_kind || null,
						firstSessionKey,
					),
				{ timeout: 10_000 },
			)
			.toBe("acp-copilot");
		await expect(picker).toBeEnabled();
		expect(pageErrors).toEqual([]);
	});

	test("ACP-only sessions auto-bind once", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await mockExternalAgentsRpc(
			page,
			[{ kind: "acp-copilot", name: "ACP: Copilot", installed: true, isAcp: true, version: null }],
			[],
		);
		await page.goto("/chats");
		await expectPageContentMounted(page);
		await waitForWsConnected(page);
		await createSession(page);

		const sessionKey = await page.evaluate(() => window.__moltis_stores?.sessionStore?.activeSessionKey?.value || "");
		await expect
			.poll(
				async () =>
					page.evaluate(
						(key) =>
							(window.__externalAgentE2ERequests || []).filter(
								(req) => req.method === "external_agents.bind" && req.params?.sessionKey === key,
							).length,
						sessionKey,
					),
				{ timeout: 10_000 },
			)
			.toBe(1);
		await expect(page.locator("#modelComboLabel")).toHaveText("ACP: Copilot");
		await expect(page.locator("#modelComboBtn")).toBeEnabled();
		expect(pageErrors).toEqual([]);
	});

	test("ACP-only sessions continue retrying failed auto-bind attempts", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await mockExternalAgentsRpc(
			page,
			[{ kind: "acp-copilot", name: "ACP: Copilot", installed: true, isAcp: true, version: null }],
			[],
			3,
		);
		await page.goto("/chats");
		await expectPageContentMounted(page);
		await waitForWsConnected(page);

		const sessionKey = await page.evaluate(() => window.__moltis_stores?.sessionStore?.activeSessionKey?.value || "");
		await expect
			.poll(
				async () =>
					page.evaluate(
						(key) =>
							(window.__externalAgentE2ERequests || []).filter(
								(req) => req.method === "external_agents.bind" && req.params?.sessionKey === key,
							).length,
						sessionKey,
					),
				{ timeout: 10_000 },
			)
			.toBe(4);
		await expect(page.locator("#modelComboLabel")).toHaveText("ACP: Copilot");
		expect(pageErrors).toEqual([]);
	});

	test("create form validates required fields", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/agents");

		await page.getByRole("button", { name: "New Agent", exact: true }).click();
		await expect(page.getByText("Create Agent", { exact: true })).toBeVisible();

		// Submit with empty fields
		await page.getByRole("button", { name: "Create", exact: true }).click();
		await expect(page.getByText("Name is required.", { exact: true })).toBeVisible();

		// Fill name but not ID
		await page.getByPlaceholder("Creative Writer").fill("Test");
		await page.getByRole("button", { name: "Create", exact: true }).click();
		await expect(page.getByText("ID is required.", { exact: true })).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("Edit button on main agent opens edit form", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/agents");

		const mainCard = page.locator('.backend-card[data-agent-id="main"]');
		await mainCard.getByRole("button", { name: "Edit", exact: true }).click();

		// The edit form should appear (heading begins with "Edit")
		await expect(page.getByText(/^Edit\s/, { exact: false })).toBeVisible({ timeout: 10_000 });

		expect(pageErrors).toEqual([]);
	});

	test("shows workspace prompt truncation warning when AGENTS.md exceeds the cap", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/agents");
		await waitForWsConnected(page);

		const originalResponse = await sendRpcFromPage(page, "agents.files.get", {
			agent_id: "main",
			path: "AGENTS.md",
		});
		const originalContent = originalResponse?.ok ? originalResponse.payload?.content || "" : "";
		const oversizedContent = `${"A".repeat(32_050)}\n`;

		try {
			const setResponse = await sendRpcFromPage(page, "agents.files.set", {
				agent_id: "main",
				path: "AGENTS.md",
				content: oversizedContent,
			});
			expect(setResponse?.ok).toBe(true);

			await navigateAndWait(page, "/settings/agents");
			const mainCard = page.locator('.backend-card[data-agent-id="main"]');
			await expect(mainCard).toContainText("AGENTS.md", { timeout: 10_000 });
			await expect(mainCard).toContainText("truncated by", { timeout: 10_000 });
		} finally {
			await sendRpcFromPage(page, "agents.files.set", {
				agent_id: "main",
				path: "AGENTS.md",
				content: originalContent,
			});
		}

		expect(pageErrors).toEqual([]);
	});
});

test.describe("Welcome card agent picker", () => {
	test("welcome card shows main agent chip and hatch button with one agent", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		// Navigate to a new session and wait for whichever empty chat card is valid for this runtime.
		await page.goto("/chats");
		await expectPageContentMounted(page);
		await waitForWsConnected(page);
		await createSession(page);

		const welcomeCard = await waitForWelcomeOrNoProvidersCard(page);
		if (!welcomeCard) {
			expect(pageErrors).toEqual([]);
			return;
		}

		// Agent chips container should be visible with main chip + hatch button
		const agentsContainer = page.locator("[data-welcome-agents]");
		await expect(agentsContainer).toBeVisible();

		// The "Hatch a new agent" discovery button should be present
		await expect(agentsContainer.getByRole("button", { name: /Hatch a new agent/ })).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("hatch button navigates to agents page with create form open", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await page.goto("/chats");
		await expectPageContentMounted(page);
		await waitForWsConnected(page);
		await createSession(page);

		const welcomeCard = await waitForWelcomeOrNoProvidersCard(page);
		if (!welcomeCard) {
			expect(pageErrors).toEqual([]);
			return;
		}

		// Click the "Hatch a new agent" button
		const hatchBtn = page.locator("[data-welcome-agents]").getByRole("button", { name: /Hatch a new agent/ });
		await expect(hatchBtn).toBeVisible();
		await hatchBtn.click();

		// Should navigate to /settings/agents/new and auto-open the create form
		await expect(page).toHaveURL(/\/settings\/agents\/new/);
		await expect(page.getByText("Create Agent", { exact: true })).toBeVisible({ timeout: 10_000 });

		expect(pageErrors).toEqual([]);
	});

	test("agent chips appear on welcome card when multiple agents exist", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		const testAgentName = "Welcome Test Agent";

		// Create a second agent via the settings page
		await navigateAndWait(page, "/settings/agents");
		await waitForWsConnected(page);

		await page.getByRole("button", { name: "New Agent", exact: true }).click();
		await expect(page.getByText("Create Agent", { exact: true })).toBeVisible();

		await page.getByPlaceholder("e.g. writer, coder, researcher").fill("welcome-test");
		await page.getByPlaceholder("Creative Writer").fill(testAgentName);
		await page.getByRole("button", { name: "Create", exact: true }).click();

		// Wait for the agent to appear in the list
		await expect(page.getByRole("heading", { name: "Agents", exact: true })).toBeVisible({ timeout: 10_000 });
		await expect(page.locator(".backend-card").filter({ hasText: testAgentName })).toBeVisible();

		// Navigate to chats and create a new session — welcome card should show agent chips
		await page.goto("/chats");
		await expectPageContentMounted(page);
		await createSession(page);

		const welcomeCard = await waitForWelcomeOrNoProvidersCard(page);
		if (!welcomeCard) {
			await deleteAgentByName(page, testAgentName);
			expect(pageErrors).toEqual([]);
			return;
		}

		const agentsContainer = page.locator("[data-welcome-agents]");
		await expect(agentsContainer).toBeVisible({ timeout: 10_000 });

		// Should have at least 2 chip buttons (main + the new agent)
		const chips = agentsContainer.getByRole("button");
		const chipCount = await chips.count();
		expect(chipCount).toBeGreaterThanOrEqual(2);
		await expect(agentsContainer.getByRole("button", { name: new RegExp(testAgentName) })).toBeVisible();

		// Clean up: delete the test agent
		await deleteAgentByName(page, testAgentName);

		expect(pageErrors).toEqual([]);
	});
});
