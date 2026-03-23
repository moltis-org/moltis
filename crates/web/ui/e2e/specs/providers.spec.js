const { expect, test } = require("../base-test");
const { navigateAndWait, watchPageErrors } = require("../helpers");

async function openProvidersPage(page) {
	await navigateAndWait(page, "/settings/providers");
	await expect.poll(() => new URL(page.url()).pathname).toBe("/settings/providers");
	await expect(page.locator("#providersTitle")).toBeVisible();
}

test.describe("Provider setup page", () => {
	test("provider page loads", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await openProvidersPage(page);
		expect(pageErrors).toEqual([]);
	});

	test("add provider button exists", async ({ page }) => {
		await openProvidersPage(page);
		await expect(page.locator("#providersAddLlmBtn")).toBeVisible();
	});

	test("detect models button exists", async ({ page }) => {
		await openProvidersPage(page);
		await expect(page.locator("#providersDetectModelsBtn")).toBeVisible();
	});

	test("no providers shows guidance", async ({ page }) => {
		await openProvidersPage(page);

		// On a fresh server with no API keys, should show guidance or empty state
		const content = page.locator("#pageContent");
		await expect(content).not.toBeEmpty();
	});

	test("page has no JS errors", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await openProvidersPage(page);
		expect(pageErrors).toEqual([]);
	});

	test("provider modal honors configured provider order", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await openProvidersPage(page);
		await page.locator("#providersAddLlmBtn").click();

		const providerNames = page.locator(".provider-modal-backdrop .provider-item .provider-item-name");
		await expect(providerNames.first()).toBeVisible();
		const names = await providerNames.allTextContents();
		const preferredOrder = ["Local LLM (Offline)", "GitHub Copilot", "OpenAI", "Anthropic", "Ollama"];
		const expectedVisible = preferredOrder.filter((name) => names.includes(name));
		const actualVisible = names.filter((name) => expectedVisible.includes(name));
		expect(actualVisible).toEqual(expectedVisible);
		expect(pageErrors).toEqual([]);
	});

	test("api key forms include provider key source hints", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await openProvidersPage(page);
		await page.locator("#providersAddLlmBtn").click();

		const openaiItem = page
			.locator(".provider-modal-backdrop .provider-item")
			.filter({ has: page.locator(".provider-item-name", { hasText: /^OpenAI$/ }) })
			.first();
		await expect(openaiItem).toBeVisible();
		await openaiItem.click();

		await expect(page.getByRole("link", { name: "OpenAI Platform" })).toBeVisible();
		await page.getByRole("button", { name: "Back", exact: true }).click();

		const optionalCandidates = [
			{ providerName: "Kimi Code", linkName: "Kimi Code Console" },
			{ providerName: "Anthropic", linkName: "Anthropic Console" },
			{ providerName: "Moonshot", linkName: "Moonshot Platform" },
		];
		for (const candidate of optionalCandidates) {
			const item = page
				.locator(".provider-modal-backdrop .provider-item")
				.filter({ has: page.locator(".provider-item-name", { hasText: new RegExp(`^${candidate.providerName}$`) }) });
			if ((await item.count()) === 0) continue;

			await item.click();
			await expect(page.getByRole("link", { name: candidate.linkName })).toBeVisible();
			await page.getByRole("button", { name: "Back", exact: true }).click();
		}

		expect(pageErrors).toEqual([]);
	});

	test("provider validation errors render in danger panel", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await openProvidersPage(page);
		await page.locator("#providersAddLlmBtn").click();

		const openaiItem = page
			.locator(".provider-modal-backdrop .provider-item")
			.filter({ has: page.locator(".provider-item-name", { hasText: /^OpenAI$/ }) })
			.first();
		await expect(openaiItem).toBeVisible();
		await openaiItem.click();

		await page.getByRole("button", { name: "Save & Validate", exact: true }).click();

		const errorPanel = page.locator(".provider-modal-backdrop .alert-error-text");
		await expect(errorPanel).toBeVisible();
		await expect(errorPanel).toContainText("Error:");
		await expect(errorPanel).toContainText("API key is required");

		expect(pageErrors).toEqual([]);
	});

	test("custom local model download progress modal renders without JS errors", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await openProvidersPage(page);

		await page.evaluate(async () => {
			const [providers, events, state] = await Promise.all([
				import("/assets/js/providers.js"),
				import("/assets/js/events.js"),
				import("/assets/js/state.js"),
			]);

			state.setWs({
				readyState: WebSocket.OPEN,
				send(raw) {
					const frame = JSON.parse(raw);
					const respond = (payload) => {
						queueMicrotask(() => {
							const pending = state.pending[frame.id];
							if (!pending) return;
							pending(payload);
							delete state.pending[frame.id];
						});
					};

					if (frame.method === "providers.local.status") {
						respond({ ok: true, payload: { status: "ready", model_id: "custom-test" } });
						return;
					}

					if (frame.method === "models.list" || frame.method === "providers.available") {
						respond({ ok: true, payload: [] });
						return;
					}

					respond({ ok: true, payload: {} });
				},
			});

			providers.showModelDownloadProgress({ id: "custom-test", displayName: "Custom.gguf" }, { name: "local-llm" });

			const listeners = events.eventListeners["local-llm.download"] || [];
			for (const listener of listeners.slice()) {
				listener({
					modelId: "custom-test",
					progress: 50,
					downloaded: 50 * 1024 * 1024,
					total: 100 * 1024 * 1024,
				});
			}
		});

		await expect(page.getByText("Downloading Custom.gguf...", { exact: true })).toBeVisible();
		await expect(page.getByText("50.0 MB / 100.0 MB", { exact: true })).toBeVisible();
		await expect
			.poll(async () => page.locator("#providerModalBody").textContent())
			.toContain("Custom.gguf configured successfully!");
		expect(pageErrors).toEqual([]);
	});
});
