const { expect, test } = require("@playwright/test");
const { navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

function isRetryableRpcError(message) {
	if (typeof message !== "string") return false;
	return message.includes("WebSocket not connected") || message.includes("WebSocket disconnected");
}

async function sendRpcFromPage(page, method, params) {
	let lastResponse = null;
	for (let attempt = 0; attempt < 30; attempt++) {
		if (attempt > 0) {
			await waitForWsConnected(page);
		}
		lastResponse = await page
			.evaluate(
				async ({ methodName, methodParams }) => {
					var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
					if (!appScript) throw new Error("app module script not found");
					var appUrl = new URL(appScript.src, window.location.origin);
					var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
					var helpers = await import(`${prefix}js/helpers.js`);
					return helpers.sendRpc(methodName, methodParams);
				},
				{
					methodName: method,
					methodParams: params,
				},
			)
			.catch((error) => ({ ok: false, error: { message: error?.message || String(error) } }));

		if (lastResponse?.ok) return lastResponse;
		if (!isRetryableRpcError(lastResponse?.error?.message)) return lastResponse;
	}
	return lastResponse;
}

async function waitForChatInputReady(page) {
	const chatInput = page.locator("#chatInput");
	await expect(chatInput).toBeVisible({ timeout: 15_000 });
	await expect(chatInput).toBeEnabled();
	return chatInput;
}

async function setChatSeq(page, seq) {
	await page.evaluate(async (nextSeq) => {
		var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
		if (!appScript) throw new Error("app module script not found");
		var appUrl = new URL(appScript.src, window.location.origin);
		var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
		var state = await import(`${prefix}js/state.js`);
		state.setChatSeq(nextSeq);
	}, seq);
}

async function getChatSeq(page) {
	return await page.evaluate(async () => {
		var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
		if (!appScript) throw new Error("app module script not found");
		var appUrl = new URL(appScript.src, window.location.origin);
		var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
		var state = await import(`${prefix}js/state.js`);
		return state.chatSeq;
	});
}

async function openFullContextWithRetry(page) {
	const toggleBtn = page.locator("#fullContextBtn");
	const panel = page.locator("#fullContextPanel");
	const copyBtn = panel.getByRole("button", { name: "Copy", exact: true });
	const failedMsg = panel.getByText("Failed to build context", { exact: true });

	for (let attempt = 0; attempt < 5; attempt++) {
		await waitForWsConnected(page);
		const fullContextRpc = await sendRpcFromPage(page, "chat.full_context", {});
		const noProvidersConfigured =
			fullContextRpc?.error?.message?.includes("no LLM providers configured") ||
			fullContextRpc?.error?.message?.includes("chat not configured");

		const panelVisible = await panel.isVisible().catch(() => false);
		if (panelVisible) {
			await toggleBtn.click();
		}

		await toggleBtn.click();
		await expect(panel).toBeVisible();

		const result = await expect
			.poll(
				async () => {
					if (await copyBtn.isVisible().catch(() => false)) return "copy";
					if (await failedMsg.isVisible().catch(() => false)) return "failed";
					return "loading";
				},
				{ timeout: 4_000 },
			)
			.toBe("copy")
			.then(() => "copy")
			.catch(() => "failed");

		if (result === "copy") return copyBtn;
		if (result === "failed" && noProvidersConfigured) {
			return null;
		}
	}

	return false;
}

async function runClearSlashCommandWithRetry(page) {
	const chatInput = page.locator("#chatInput");
	for (let attempt = 0; attempt < 6; attempt++) {
		await waitForWsConnected(page);
		await waitForChatInputReady(page);
		await chatInput.click();
		await chatInput.fill("/clear");
		await expect(chatInput).toHaveValue("/clear");
		await chatInput.press("Enter");
		const reset = await expect
			.poll(async () => await getChatSeq(page), { timeout: 4_000 })
			.toBe(0)
			.then(() => true)
			.catch(() => false);
		if (reset) return true;
		// Recover test state so the next slash-command attempt starts cleanly.
		await sendRpcFromPage(page, "chat.clear", {});
		await setChatSeq(page, 8);
	}
	return false;
}

test.describe("Chat input and slash commands", () => {
	test.beforeEach(async ({ page }) => {
		await navigateAndWait(page, "/chats/main");
		await waitForWsConnected(page);
		await waitForChatInputReady(page);
	});

	test("chat input is visible and focusable", async ({ page }) => {
		const chatInput = page.locator("#chatInput");
		await expect(chatInput).toBeVisible();
		await chatInput.focus();
		await expect(chatInput).toBeFocused();
	});

	test('typing "/" shows slash command menu', async ({ page }) => {
		const chatInput = page.locator("#chatInput");
		await chatInput.focus();
		await chatInput.fill("/");

		const slashMenu = page.locator(".slash-menu");
		await expect(slashMenu).toBeVisible({ timeout: 5_000 });

		// Should have at least one menu item
		const items = slashMenu.locator(".slash-menu-item");
		await expect
			.poll(async () => await items.count(), {
				timeout: 10_000,
			})
			.toBeGreaterThan(0);
	});

	test("slash menu filters as user types", async ({ page }) => {
		const chatInput = page.locator("#chatInput");
		await chatInput.focus();
		await chatInput.fill("/");

		const slashMenu = page.locator(".slash-menu");
		await expect(slashMenu).toBeVisible({ timeout: 5_000 });

		const countAll = await slashMenu.locator(".slash-menu-item").count();

		// Type more to filter
		await chatInput.fill("/cl");
		await expect
			.poll(async () => await slashMenu.locator(".slash-menu-item").count(), {
				timeout: 5_000,
			})
			.toBeLessThanOrEqual(countAll);
	});

	test("Escape dismisses slash menu", async ({ page }) => {
		const chatInput = page.locator("#chatInput");
		await chatInput.focus();
		await chatInput.fill("/");

		const slashMenu = page.locator(".slash-menu");
		await expect(slashMenu).toBeVisible({ timeout: 5_000 });

		await page.keyboard.press("Escape");
		await expect(slashMenu).toBeHidden();
	});

	test("Shift+Enter inserts newline without sending", async ({ page }) => {
		const chatInput = page.locator("#chatInput");
		await chatInput.focus();
		await chatInput.fill("line one");
		await page.keyboard.press("Shift+Enter");
		await page.keyboard.type("line two");

		const value = await chatInput.inputValue();
		expect(value).toContain("line one");
		expect(value).toContain("line two");
	});

	test("model selector dropdown opens and closes", async ({ page }) => {
		const modelBtn = page.locator("#modelComboBtn");
		if (await modelBtn.isVisible()) {
			await modelBtn.click();

			const dropdown = page.locator("#modelDropdown");
			await expect(dropdown).toBeVisible();

			// Close by clicking button again
			await modelBtn.click();
			await expect(dropdown).toBeHidden();
		}
	});

	test("send button is present", async ({ page }) => {
		const sendBtn = page.locator("#sendBtn");
		await expect(sendBtn).toBeVisible();
	});

	test("audio duration formatter handles invalid values", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		const formatted = await page.evaluate(async () => {
			var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			if (!appScript) throw new Error("app module script not found");
			var appUrl = new URL(appScript.src, window.location.origin);
			var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			var helpers = await import(`${prefix}js/helpers.js`);
			return {
				nan: helpers.formatAudioDuration(Number.NaN),
				inf: helpers.formatAudioDuration(Number.POSITIVE_INFINITY),
				short: helpers.formatAudioDuration(2.4),
			};
		});

		expect(formatted.nan).toBe("00:00");
		expect(formatted.inf).toBe("00:00");
		expect(formatted.short).toBe("00:02");
		expect(pageErrors).toEqual([]);
	});

	test("prompt button is hidden from chat header", async ({ page }) => {
		await expect(page.locator("#rawPromptBtn")).toHaveCount(0);
	});

	test("full context copy button uses small button style", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		const copyBtn = await openFullContextWithRetry(page);
		if (copyBtn === null) {
			await expect(
				page.locator("#fullContextPanel").getByText("Failed to build context", { exact: true }),
			).toBeVisible();
			expect(pageErrors).toEqual([]);
			return;
		}
		expect(copyBtn).not.toBe(false);
		expect(copyBtn).not.toBeNull();
		await expect(copyBtn).toBeVisible();
		await expect(copyBtn).toHaveClass(/provider-btn-sm/);
		expect(pageErrors).toEqual([]);
	});

	test("/clear resets client chat sequence", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await setChatSeq(page, 8);

		const reset = await runClearSlashCommandWithRetry(page);
		expect(reset).toBeTruthy();
		expect(pageErrors).toEqual([]);
	});
});
