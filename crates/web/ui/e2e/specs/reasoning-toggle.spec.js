const { expect, test } = require("../base-test");
const { navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

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
			// Set effort AFTER models so supportsReasoning is true; the effect
			// in reasoning-toggle resets effort to "" when supportsReasoning=false.
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
		await expect(items).toHaveCount(4);
		await expect(items.nth(0)).toHaveText("Off");
		await expect(items.nth(1)).toHaveText("Low");
		await expect(items.nth(2)).toHaveText("Medium");
		await expect(items.nth(3)).toHaveText("High");

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
		const highItem = page.locator("#reasoningDropdownList .model-dropdown-item", { hasText: "High" });
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

		const items = page.locator("#modelDropdownList .model-dropdown-item");
		// Only the base model should appear, not the 3 reasoning variants
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
