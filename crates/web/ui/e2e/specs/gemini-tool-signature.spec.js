const http = require("node:http");

const { expect, test } = require("../base-test");
const { navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

const MODEL_ID = "gemini-e2e-tool-signature";
const SIGNATURE = "sig_e2e_issue_375";
const SENTINEL = "GEMINI_E2E_TOOL_SIGNATURE_OK";
const MISSING_SIGNATURE_ERROR =
	"Function call is missing a thought_signature in functionCall parts. This is required for tools to work correctly, and missing thought_signature may lead to degraded model performance.";

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

function readRequestJson(req) {
	return new Promise((resolve, reject) => {
		let raw = "";
		req.setEncoding("utf8");
		req.on("data", (chunk) => {
			raw += chunk;
		});
		req.on("end", () => {
			try {
				resolve(raw ? JSON.parse(raw) : {});
			} catch (_error) {
				reject(new Error("invalid request json"));
			}
		});
		req.on("error", reject);
	});
}

function writeJson(res, status, body) {
	res.writeHead(status, { "content-type": "application/json" });
	res.end(JSON.stringify(body));
}

function writeSse(res, events) {
	res.writeHead(200, {
		"cache-control": "no-cache",
		connection: "keep-alive",
		"content-type": "text/event-stream",
	});
	for (const event of events) {
		res.write(`data: ${typeof event === "string" ? event : JSON.stringify(event)}\n\n`);
	}
	res.end();
}

function toolCallStartEvent() {
	return {
		choices: [
			{
				delta: {
					tool_calls: [
						{
							index: 0,
							id: "gemini_call_1",
							thought_signature: SIGNATURE,
							function: {
								name: "exec",
								arguments: "",
							},
						},
					],
				},
				finish_reason: null,
			},
		],
	};
}

function toolCallArgumentsEvent() {
	return {
		choices: [
			{
				delta: {
					tool_calls: [
						{
							index: 0,
							function: {
								arguments: JSON.stringify({ command: `printf ${SENTINEL}` }),
							},
						},
					],
				},
				finish_reason: "tool_calls",
			},
		],
	};
}

function finalTextEvent() {
	return {
		choices: [
			{
				delta: {
					content: SENTINEL,
				},
				finish_reason: null,
			},
		],
	};
}

function doneEvent() {
	return {
		choices: [
			{
				delta: {},
				finish_reason: "stop",
			},
		],
		usage: {
			prompt_tokens: 12,
			completion_tokens: 4,
			total_tokens: 16,
		},
	};
}

function requestIncludesThoughtSignature(body) {
	const messages = Array.isArray(body?.messages) ? body.messages : [];
	return messages.some((message) => {
		const toolCalls = Array.isArray(message?.tool_calls) ? message.tool_calls : [];
		return toolCalls.some((toolCall) => toolCall?.thought_signature === SIGNATURE);
	});
}

async function startGeminiMockServer() {
	const completionRequests = [];
	let completionCount = 0;
	const server = http.createServer(async (req, res) => {
		try {
			if (req.method === "GET" && req.url === "/v1/models") {
				writeJson(res, 200, {
					object: "list",
					data: [{ id: MODEL_ID, object: "model", created: 1, owned_by: "e2e" }],
				});
				return;
			}

			if (req.method !== "POST" || req.url !== "/v1/chat/completions") {
				writeJson(res, 404, { error: { message: "not found" } });
				return;
			}

			const body = await readRequestJson(req);
			completionCount += 1;
			completionRequests.push(body);

			if (completionCount === 1) {
				writeSse(res, [toolCallStartEvent(), toolCallArgumentsEvent(), "[DONE]"]);
				return;
			}

			if (!requestIncludesThoughtSignature(body)) {
				writeJson(res, 400, {
					error: {
						code: 400,
						message: MISSING_SIGNATURE_ERROR,
						status: "INVALID_ARGUMENT",
					},
				});
				return;
			}

			writeSse(res, [finalTextEvent(), doneEvent(), "[DONE]"]);
		} catch (_error) {
			writeJson(res, 500, { error: { message: "mock server internal error" } });
		}
	});

	await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
	const address = server.address();
	if (!address || typeof address === "string") {
		throw new Error("mock server did not bind to a TCP port");
	}

	return {
		baseUrl: `http://127.0.0.1:${address.port}/v1`,
		completionRequests,
		close: () => new Promise((resolve, reject) => server.close((error) => (error ? reject(error) : resolve()))),
	};
}

test.describe("Gemini tool-call thought signatures", () => {
	test("web chat round-trips tool call metadata into the follow-up provider request", async ({ page }) => {
		test.setTimeout(120_000);
		const mock = await startGeminiMockServer();
		const pageErrors = watchPageErrors(page);
		let providerName = "";

		try {
			await navigateAndWait(page, "/");
			await waitForWsConnected(page);

			const customResponse = await expectRpcOk(page, "providers.add_custom", {
				baseUrl: mock.baseUrl,
				apiKey: "e2e-key",
				model: MODEL_ID,
			});
			providerName = String(customResponse.payload?.providerName || "");
			expect(providerName).toMatch(/^custom-/);

			const modelsResponse = await expectRpcOk(page, "models.list", {});
			const model = (modelsResponse.payload || []).find((entry) => entry?.id === `${providerName}::${MODEL_ID}`);
			expect(model, "expected the custom Gemini mock model to be registered").toBeTruthy();

			// Abort any stale run so the session semaphore is free.
			// There is a brief race between abort returning and the
			// spawned task dropping the permit, so we poll chat.send
			// until we get a runId (cancelling queued messages between
			// attempts to avoid duplicates).
			await sendRpcFromPage(page, "chat.abort", { sessionKey: "main" });
			await expectRpcOk(page, "chat.clear", { sessionKey: "main" });

			let sendResponse;
			for (let attempt = 0; attempt < 10; attempt++) {
				sendResponse = await sendRpcFromPage(page, "chat.send", {
					sessionKey: "main",
					model: model.id,
					text: "Use the exec tool to print the sentinel.",
				});
				if (sendResponse?.ok && sendResponse.payload?.runId) break;
				// Message was queued — cancel it and retry after a short wait.
				await sendRpcFromPage(page, "chat.cancel_queued", { sessionKey: "main" });
				await page.waitForTimeout(200);
			}
			expect(sendResponse?.ok, "chat.send should succeed").toBeTruthy();
			expect(String(sendResponse.payload?.runId || "")).not.toBe("");

			await expect
				.poll(
					async () => {
						const historyResponse = await sendRpcFromPage(page, "chat.history", { sessionKey: "main" });
						if (!historyResponse?.ok) {
							return `history-rpc-error:${historyResponse?.error?.message || "unknown error"}`;
						}

						const errorCards = page.locator(".error-card, .msg.error");
						const errorCount = await errorCards.count();
						if (errorCount > 0) {
							const text = await errorCards
								.nth(errorCount - 1)
								.textContent()
								.catch(() => "");
							if (text) return `page-error:${text.trim()}`;
						}

						const assistantMessages = (historyResponse.payload || []).filter((message) => message.role === "assistant");
						return String(assistantMessages.at(-1)?.content || "");
					},
					{ timeout: 120_000 },
				)
				.toContain(SENTINEL);

			expect(mock.completionRequests.length).toBeGreaterThanOrEqual(2);
			expect(requestIncludesThoughtSignature(mock.completionRequests[1])).toBe(true);

			const historyResponse = await expectRpcOk(page, "chat.history", { sessionKey: "main" });
			const persistedToolCall = (historyResponse.payload || [])
				.flatMap((message) => (Array.isArray(message?.tool_calls) ? message.tool_calls : []))
				.find((toolCall) => toolCall?.id === "gemini_call_1");
			expect(persistedToolCall?.metadata?.thought_signature).toBe(SIGNATURE);

			await expect(page.locator(".error-card, .msg.error")).toHaveCount(0);
			expect(pageErrors).toEqual([]);
		} finally {
			if (providerName) {
				await sendRpcFromPage(page, "providers.remove_key", { provider: providerName }).catch(() => null);
			}
			await mock.close();
		}
	});
});
