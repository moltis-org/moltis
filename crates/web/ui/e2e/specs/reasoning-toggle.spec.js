const { expect, test } = require("../base-test");
const {
	createSession,
	navigateAndWait,
	sendRpcFromPage,
	waitForChatSessionReady,
	waitForWsConnected,
	watchPageErrors,
} = require("../helpers");

/** Set mock models in the browser and freeze the store so bootstrap/WS cannot overwrite. */
async function setMockModels(page, models, selectedId, effort) {
	await page.evaluate(
		async ([models, selectedId, effort]) => {
			var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			var appUrl = new URL(appScript.src, window.location.origin);
			var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			var store = await import(`${prefix}js/stores/model-store.js`);
			// Wait for bootstrap to populate the initial model list so there are
			// no in-flight fetch/setAll calls that could overwrite our mock data.
			for (var i = 0; i < 100 && store.models.value.length === 0; i++) {
				await new Promise((r) => setTimeout(r, 50));
			}
			// Freeze the modelStore object to block future bootstrap/WS updates.
			store.modelStore.fetch = () => Promise.resolve();
			store.modelStore.setAll = () => {};
			// Select the model ID BEFORE setting the list so that when the models
			// signal updates, the computed selectedModel/supportsReasoning resolve
			// immediately without a brief "model not found" gap.
			store.select(selectedId);
			store.setAll(models);
			// Set an explicit effort only when the test requests one.
			if (effort) store.setReasoningEffort(effort);
		},
		[models, selectedId, effort],
	);
}

test.describe("reasoning effort toggle", () => {
	test.beforeEach(async ({ page }) => {
		await navigateAndWait(page, "/");
		await waitForWsConnected(page);
	});

	test("reasoning combo is hidden when model does not support reasoning", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await setMockModels(
			page,
			[{ id: "gpt-4o", displayName: "GPT-4o", provider: "openai", supportsReasoning: false }],
			"gpt-4o",
		);

		const reasoningCombo = page.locator("#reasoningCombo");
		await expect(reasoningCombo).toBeHidden();
		expect(pageErrors).toEqual([]);
	});

	test("reasoning combo appears when model supports reasoning", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await setMockModels(
			page,
			[{ id: "claude-opus-4-5", displayName: "Claude Opus 4.5", provider: "anthropic", supportsReasoning: true }],
			"claude-opus-4-5",
		);

		const reasoningCombo = page.locator("#reasoningCombo");
		await expect(reasoningCombo).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("clicking toggle opens dropdown with Off/Low/Medium/High options", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await setMockModels(
			page,
			[{ id: "claude-opus-4-5", displayName: "Claude Opus 4.5", provider: "anthropic", supportsReasoning: true }],
			"claude-opus-4-5",
		);

		const comboBtn = page.locator("#reasoningComboBtn");
		await expect(comboBtn).toBeVisible();
		await comboBtn.click();

		const dropdown = page.locator("#reasoningDropdown");
		await expect(dropdown).toBeVisible();

		const items = page.locator("#reasoningDropdownList .model-dropdown-item");
		await expect(items).toHaveCount(6);
		await expect(items.nth(0)).toHaveText("Off");
		await expect(items.nth(1)).toHaveText("Minimal");
		await expect(items.nth(2)).toHaveText("Low");
		await expect(items.nth(3)).toHaveText("Medium");
		await expect(items.nth(4)).toHaveText("High");
		await expect(items.nth(5)).toHaveText("Extra High");

		expect(pageErrors).toEqual([]);
	});

	test("selecting effort level updates label and closes dropdown", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await setMockModels(
			page,
			[{ id: "claude-opus-4-5", displayName: "Claude Opus 4.5", provider: "anthropic", supportsReasoning: true }],
			"claude-opus-4-5",
		);

		const comboBtn = page.locator("#reasoningComboBtn");
		await expect(comboBtn).toBeVisible();
		await comboBtn.click();

		// Wait for dropdown to be visible before selecting
		const highItem = page.locator("#reasoningDropdownList .model-dropdown-item").filter({ hasText: /^High$/ });
		await expect(highItem).toBeVisible();
		await highItem.click();

		const dropdown = page.locator("#reasoningDropdown");
		await expect(dropdown).toBeHidden();

		const label = page.locator("#reasoningComboLabel");
		await expect(label).toHaveText("High");

		expect(pageErrors).toEqual([]);
	});

	test("effective model ID includes reasoning suffix in chat.send", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		// Install WS spy to capture chat.send payloads
		await page.evaluate(() => {
			window.__chatSendPayloads = [];
			if (window.__chatWsSpyInstalled) return;
			var originalSend = WebSocket.prototype.send;
			WebSocket.prototype.send = function (data) {
				try {
					var parsed = JSON.parse(data);
					if (parsed?.method === "chat.send") {
						window.__chatSendPayloads.push(parsed.params || {});
					}
				} catch {
					// ignore non-JSON payloads
				}
				return originalSend.call(this, data);
			};
			window.__chatWsSpyInstalled = true;
		});

		// Set up a reasoning model and select high effort
		await setMockModels(
			page,
			[{ id: "claude-opus-4-5", displayName: "Claude Opus 4.5", provider: "anthropic", supportsReasoning: true }],
			"claude-opus-4-5",
			"high",
		);

		const chatInput = page.locator("#chatInput");
		await chatInput.fill("hello");
		await chatInput.press("Enter");

		const payloads = await page.evaluate(() => window.__chatSendPayloads);
		expect(payloads.length).toBeGreaterThan(0);
		expect(payloads[0].model).toBe("claude-opus-4-5@reasoning-high");

		expect(pageErrors).toEqual([]);
	});

	test("reasoning variants are filtered from model dropdown", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await setMockModels(
			page,
			[
				{
					id: "claude-opus-4-5",
					displayName: "Claude Opus 4.5",
					provider: "anthropic",
					supportsReasoning: true,
				},
				{
					id: "claude-opus-4-5@reasoning-low",
					displayName: "Claude Opus 4.5 (low reasoning)",
					provider: "anthropic",
					supportsReasoning: true,
				},
				{
					id: "claude-opus-4-5@reasoning-medium",
					displayName: "Claude Opus 4.5 (medium reasoning)",
					provider: "anthropic",
					supportsReasoning: true,
				},
				{
					id: "claude-opus-4-5@reasoning-high",
					displayName: "Claude Opus 4.5 (high reasoning)",
					provider: "anthropic",
					supportsReasoning: true,
				},
			],
			"claude-opus-4-5",
		);

		const modelBtn = page.locator("#modelComboBtn");
		await modelBtn.click();

		const items = page.locator("#modelDropdownList .model-dropdown-item[data-model-id]");
		// Only the base model should appear among the model entries, not the 3 reasoning variants.
		await expect(items).toHaveCount(1);
		await expect(items.first()).toContainText("Claude Opus 4.5");

		expect(pageErrors).toEqual([]);
	});

	test("switching to non-reasoning model resets effort to Off", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await setMockModels(
			page,
			[
				{
					id: "claude-opus-4-5",
					displayName: "Claude Opus 4.5",
					provider: "anthropic",
					supportsReasoning: true,
				},
				{ id: "gpt-4o", displayName: "GPT-4o", provider: "openai", supportsReasoning: false },
			],
			"claude-opus-4-5",
			"high",
		);

		// Verify reasoning is High
		const label = page.locator("#reasoningComboLabel");
		await expect(label).toHaveText("High");

		// Switch to non-reasoning model
		await page.evaluate(async () => {
			var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			var appUrl = new URL(appScript.src, window.location.origin);
			var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			var store = await import(`${prefix}js/stores/model-store.js`);
			store.select("gpt-4o");
		});

		// Combo should be hidden
		const reasoningCombo = page.locator("#reasoningCombo");
		await expect(reasoningCombo).toBeHidden();

		// Effort should be reset
		const effort = await page.evaluate(async () => {
			var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			var appUrl = new URL(appScript.src, window.location.origin);
			var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			var store = await import(`${prefix}js/stores/model-store.js`);
			return store.reasoningEffort.value;
		});
		expect(effort).toBe("");

		expect(pageErrors).toEqual([]);
	});
});

test.describe("configured reasoning default", () => {
	const models = [
		{ id: "reasoning-model", displayName: "Reasoning Model", provider: "openai", supportsReasoning: true },
		{ id: "plain-model", displayName: "Plain Model", provider: "openai", supportsReasoning: false },
	];

	test.beforeEach(async ({ page }) => {
		await page.addInitScript(() => {
			let data;
			Object.defineProperty(window, "__MOLTIS__", {
				get: () => data,
				set: (value) => {
					data = { ...value, reasoning_default: "high" };
				},
				configurable: true,
			});
			localStorage.setItem("moltis-reasoning-effort", "low");
		});
		await navigateAndWait(page, "/");
		await waitForWsConnected(page);
		await waitForChatSessionReady(page);
		// The gateway is shared across tests. Remove the previous main session's
		// model override so bootstrap really exercises the configured default.
		const reset = await sendRpcFromPage(page, "sessions.patch", { key: "main", model: "" });
		expect(reset.ok).toBe(true);
		await navigateAndWait(page, "/");
		await waitForWsConnected(page);
		await waitForChatSessionReady(page);
	});

	test("main session default survives bootstrap and delayed model capabilities", async ({ page }) => {
		const errors = watchPageErrors(page);
		await setMockModels(page, models, "reasoning-model");
		await expect(page.locator("#reasoningCombo").getByRole("button")).toHaveText("High");
		await page.evaluate(() => {
			const store = window.__moltis_stores.modelStore;
			const models = store.models.value;
			store.models.value = [];
			store.models.value = models;
		});
		await expect(page.locator("#reasoningCombo").getByRole("button")).toHaveText("High");
		expect(errors).toEqual([]);
	});

	for (const [label, suffix] of [
		["Off", ""],
		["Extra High", "@reasoning-xhigh"],
	]) {
		test(`${label} is saved before sending and restored independently of new sessions`, async ({ page }) => {
			const errors = watchPageErrors(page);
			await setMockModels(page, models, "reasoning-model");
			await createSession(page);
			const key = await page.evaluate(() => window.__moltis_stores.sessionStore.activeSessionKey.value);
			await expect(page.locator("#reasoningCombo").getByRole("button")).toHaveText("High");
			await page.locator("#reasoningCombo").getByRole("button").click();
			await page.locator("#reasoningDropdownList").getByText(label, { exact: true }).click();
			await createSession(page);
			await expect(page.locator("#reasoningCombo").getByRole("button")).toHaveText("High");
			const saved = await sendRpcFromPage(page, "sessions.switch", { key, include_history: false });
			expect(saved.ok).toBe(true);
			expect(saved.payload.entry.model).toBe(`reasoning-model${suffix}`);
			await page.evaluate((key) => window.__moltis_modules.sessions.switchSession(key), key);
			await waitForChatSessionReady(page);
			await expect(page.locator("#reasoningCombo").getByRole("button")).toHaveText(label);
			expect(errors).toEqual([]);
		});
	}

	test("pending history restoration blocks effort choices until the stale snapshot is applied", async ({ page }) => {
		const errors = watchPageErrors(page);
		await setMockModels(page, models, "reasoning-model");
		const button = page.locator("#reasoningCombo").getByRole("button");
		await button.click();
		await expect(page.locator("#reasoningDropdown")).toBeVisible();

		let releaseHistory;
		const historyGate = new Promise((resolve) => {
			releaseHistory = resolve;
		});
		let historyRequested;
		const historyStarted = new Promise((resolve) => {
			historyRequested = resolve;
		});
		await page.route("**/api/sessions/main/history*", async (route) => {
			const response = await route.fetch();
			historyRequested();
			await historyGate;
			await route.fulfill({ response });
		});
		try {
			await page.evaluate(() => {
				window.__moltis_modules["stores/session-history-cache"].clearSessionHistory("main");
				window.__moltis_modules.sessions.switchSession("main");
			});
			await historyStarted;
			await expect(button).toBeDisabled();
			await expect(page.locator("#reasoningDropdown")).toBeHidden();
			// Even an already queued option click must not save an override while loading.
			await page.locator("#reasoningDropdownList").getByText("Off", { exact: true }).dispatchEvent("click");
			await expect(button).toHaveText("High");
		} finally {
			releaseHistory();
		}
		await waitForChatSessionReady(page);
		await expect(button).toBeEnabled();
		await button.click();
		await page.locator("#reasoningDropdownList").getByText("Off", { exact: true }).click();
		await expect(button).toHaveText("Off");
		const saved = await sendRpcFromPage(page, "sessions.switch", { key: "main", include_history: false });
		expect(saved.ok).toBe(true);
		expect(saved.payload.entry.model).toBe("reasoning-model");
		await page.evaluate(() => {
			const originalSend = WebSocket.prototype.send;
			WebSocket.prototype.send = function (data) {
				const frame = JSON.parse(data);
				if (frame.method === "chat.send") window.__reasoningSendModel = frame.params.model;
				return originalSend.call(this, data);
			};
		});
		await page.locator("#chatInput").fill("hello");
		await page.locator("#chatInput").press("Enter");
		await expect.poll(() => page.evaluate(() => window.__reasoningSendModel)).toBe("reasoning-model");
		expect(errors).toEqual([]);
	});

	test("unsupported models omit the suffix without erasing the default", async ({ page }) => {
		const errors = watchPageErrors(page);
		await setMockModels(page, models, "plain-model");
		await expect(page.locator("#reasoningCombo")).toBeHidden();
		expect(await page.evaluate(() => window.__moltis_stores.modelStore.effectiveModelId.value)).toBe("plain-model");
		await page.evaluate(() => window.__moltis_stores.modelStore.select("reasoning-model"));
		await expect(page.locator("#reasoningCombo").getByRole("button")).toHaveText("High");
		expect(errors).toEqual([]);
	});
});
