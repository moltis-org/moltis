const { expect, test } = require("../base-test");
const { expectRpcOk, navigateAndWait, sendRpcFromPage, waitForWsConnected, watchPageErrors } = require("../helpers");

const OPENAI_API_KEY = process.env.MOLTIS_E2E_OPENAI_API_KEY || process.env.OPENAI_API_KEY || "";
const SENTINEL = "OPENAI_LIVE_E2E_OK";

function isLikelyFunctionCallingModel(modelId) {
	if (typeof modelId !== "string") return false;
	const rawId = modelId.replace(/^openai::/, "");
	return !/(?:search|audio|realtime)-preview/i.test(rawId);
}

test.describe("Live OpenAI provider", () => {
	test.describe.configure({ mode: "serial" });

	test.skip(!OPENAI_API_KEY, "requires OPENAI_API_KEY or MOLTIS_E2E_OPENAI_API_KEY");

	test("existing env can complete a real OpenAI chat turn", async ({ page }) => {
		test.setTimeout(120_000);
		const pageErrors = watchPageErrors(page);

		await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		const modelsResponse = await expectRpcOk(page, "models.list", {});
		const openAiModels = (modelsResponse.payload || []).filter(
			(model) => typeof model?.id === "string" && model.id.startsWith("openai::") && model.supportsTools === true,
		);
		const openAiModel = openAiModels.find((model) => isLikelyFunctionCallingModel(model.id)) || openAiModels[0] || null;

		expect(
			openAiModel,
			"expected at least one detected OpenAI model with tool support from the existing env",
		).toBeTruthy();

		await expectRpcOk(page, "chat.clear", { sessionKey: "main" });

		const sendResponse = await expectRpcOk(page, "chat.send", {
			sessionKey: "main",
			model: openAiModel.id,
			text: `Reply with exactly ${SENTINEL} and nothing else.`,
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
					return String(assistantMessages.at(-1)?.content || "");
				},
				{ timeout: 120_000 },
			)
			.toContain(SENTINEL);

		const historyResponse = await expectRpcOk(page, "chat.history", { sessionKey: "main" });
		const assistantMessages = (historyResponse.payload || []).filter((message) => message.role === "assistant");
		expect(assistantMessages.length).toBeGreaterThan(0);
		expect(String(assistantMessages.at(-1)?.content || "")).toContain(SENTINEL);
		expect(assistantMessages.at(-1)?.provider).toBe("openai");
		expect(String(assistantMessages.at(-1)?.model || "")).toContain(openAiModel.id.replace(/^openai::/, ""));
		await expect(page.locator(".error-card, .msg.error")).toHaveCount(0);

		expect(pageErrors).toEqual([]);
	});
});
