const { expect, test } = require("../base-test");
const { navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

const PROVIDER_PREFIX = "custom-ollama-qwen::";
const SYSTEM_MESSAGE_ERROR = "System message must be at the beginning";

function isRetryableRpcError(message) {
	if (typeof message !== "string") return false;
	return message.includes("WebSocket not connected") || message.includes("WebSocket disconnected");
}

async function sendRpcFromPage(page, method, params) {
	let lastResponse = null;
	for (let attempt = 0; attempt < 30; attempt++) {
		if (attempt > 0) {
			await waitForWsConnected(page, 5_000).catch(() => "ignored");
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

async function expectRpcOk(page, method, params) {
	const response = await sendRpcFromPage(page, method, params);
	expect(response?.ok, `RPC ${method} failed: ${response?.error?.message || "unknown error"}`).toBeTruthy();
	return response;
}

test.describe("Ollama Qwen Live", () => {
	test.describe.configure({ mode: "serial" });

	test("custom OpenAI-compatible Qwen chat completes a real turn", async ({ page }) => {
		test.setTimeout(240_000);
		const pageErrors = watchPageErrors(page);

		await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		const modelsResponse = await expectRpcOk(page, "models.list", {});
		const qwenModel = (modelsResponse.payload || []).find(
			(model) => typeof model?.id === "string" && model.id.startsWith(PROVIDER_PREFIX),
		);

		expect(qwenModel, "expected a custom-ollama-qwen model from the seeded runtime config").toBeTruthy();

		await expectRpcOk(page, "chat.clear", { sessionKey: "main" });

		const sendResponse = await expectRpcOk(page, "chat.send", {
			sessionKey: "main",
			model: qwenModel.id,
			text: "Reply with a short JSON object containing a token key.",
		});

		expect(String(sendResponse.payload?.runId || "")).not.toBe("");

		await expect
			.poll(
				async () => {
					const historyResponse = await sendRpcFromPage(page, "chat.history", { sessionKey: "main" });
					if (!historyResponse?.ok) {
						return `history-rpc-error:${historyResponse?.error?.message || "unknown error"}`;
					}

					const pageErrorMessages = page.locator(".error-card, .msg.error");
					const pageErrorCount = await pageErrorMessages.count();
					if (pageErrorCount > 0) {
						const pageErrorText = await pageErrorMessages
							.nth(pageErrorCount - 1)
							.textContent()
							.catch(() => "");
						if (pageErrorText) {
							return `page-error:${pageErrorText.trim()}`;
						}
					}

					const assistantMessages = (historyResponse.payload || []).filter((message) => message.role === "assistant");
					return String(assistantMessages.at(-1)?.content || "").trim();
				},
				{ timeout: 240_000 },
			)
			.not.toBe("");

		const historyResponse = await expectRpcOk(page, "chat.history", { sessionKey: "main" });
		const assistantMessages = (historyResponse.payload || []).filter((message) => message.role === "assistant");
		const finalAssistantContent = String(assistantMessages.at(-1)?.content || "").trim();
		expect(assistantMessages.length).toBeGreaterThan(0);
		expect(finalAssistantContent).not.toBe("");
		expect(finalAssistantContent).not.toContain(SYSTEM_MESSAGE_ERROR);
		expect(assistantMessages.at(-1)?.provider).toBe("custom-ollama-qwen");
		expect(String(assistantMessages.at(-1)?.model || "")).toContain(qwenModel.id.replace(PROVIDER_PREFIX, ""));
		await expect(page.locator(".error-card, .msg.error")).toHaveCount(0);

		expect(pageErrors).toEqual([]);
	});
});
