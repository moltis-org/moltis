const { expect } = require("@playwright/test");

/**
 * Wait until the SPA has mounted visible content into #pageContent.
 * This is a stable cross-route readiness signal for the app shell.
 */
async function expectPageContentMounted(page) {
	await expect
		.poll(
			async () => {
				try {
					return await page.evaluate(() => {
						const el = document.getElementById("pageContent");
						if (!el) return 0;
						return el.childElementCount;
					});
				} catch (error) {
					if (isRetryableNavigationError(error)) return 0;
					throw error;
				}
			},
			{
				timeout: 10_000,
			},
		)
		.toBeGreaterThan(0);
}

/**
 * Collect uncaught page errors for later assertion.
 * Returns an array that fills as errors occur.
 *
 * Usage:
 *   const pageErrors = watchPageErrors(page);
 *   // ... interact with page ...
 *   expect(pageErrors).toEqual([]);
 */
function watchPageErrors(page) {
	const pageErrors = [];
	page.on("pageerror", (err) => pageErrors.push(err.message));
	return pageErrors;
}

/**
 * Wait for the WebSocket connection status dot to reach "connected".
 * Note: #statusText is intentionally set to "" when connected, so we
 * only check the dot's CSS class.
 */
async function waitForWsConnected(page, timeoutMs = 10_000) {
	await expect
		.poll(
			async () => {
				const statusDotConnected = await page
					.locator("#statusDot")
					.getAttribute("class")
					.then((cls) => /connected/.test(cls || ""))
					.catch(() => false);
				if (!statusDotConnected) return false;
				return page
					.evaluate(() => {
						const state = window.__moltis_state;
						return Boolean(state?.connected && state?.subscribed && state?.ws?.readyState === WebSocket.OPEN);
					})
					.catch(() => false);
			},
			{ timeout: timeoutMs },
		)
		.toBe(true);
}

function isRetryableNavigationError(error) {
	var message = error?.message || String(error || "");
	return (
		message.includes("net::ERR_ABORTED") ||
		message.includes("page.goto: Timeout") ||
		message.includes("Execution context was destroyed") ||
		message.includes("Target page, context or browser has been closed")
	);
}

/**
 * Navigate to a path, wait for SPA content to mount, and assert no errors.
 * Returns the pageErrors array for further assertions.
 */
async function navigateAndWait(page, path) {
	const pageErrors = watchPageErrors(page);
	let lastError = null;
	const gotoTimeoutMs = 10_000;
	for (let attempt = 0; attempt < 3; attempt++) {
		try {
			// Navigate to about:blank first to release any pending connections
			// from previous navigations (HTTP/1.1 has a 6-connection-per-host limit).
			if (attempt > 0) {
				await page.goto("about:blank").catch(() => undefined);
			}
			await page.goto(path, { waitUntil: "commit", timeout: gotoTimeoutMs });
			await expectPageContentMounted(page);
			return pageErrors;
		} catch (error) {
			lastError = error;
			if (isRetryableNavigationError(error)) {
				const mountedAfterError = await expectPageContentMounted(page)
					.then(() => true)
					.catch(() => false);
				if (mountedAfterError) return pageErrors;
			}
			if (!isRetryableNavigationError(error) || attempt === 2) {
				// Capture diagnostic info when navigation fails
				try {
					var testInfo = require("@playwright/test").test.info();
					var responses = [];
					var consoleMessages = [];
					page.on("response", (r) => responses.push(`${r.status()} ${r.url()}`));
					page.on("console", (m) => consoleMessages.push(`[${m.type()}] ${m.text()}`));
					// Try one more goto with a short timeout to capture what happens
					await page.goto(path, { waitUntil: "commit", timeout: gotoTimeoutMs }).catch(() => undefined);
					// Check server health to see if it's alive
					var healthOk = "unknown";
					try {
						var baseURL = testInfo.project.use?.baseURL || "http://127.0.0.1";
						// Use http module directly — page.request dies when test timeout kills the context
						var http = require("node:http");
						healthOk = await new Promise((resolve) => {
							var req = http.get(`${baseURL}/health`, { timeout: 3000 }, (res) => {
								var body = "";
								res.on("data", (d) => (body += d));
								res.on("end", () => resolve(`${res.statusCode} ${body.slice(0, 200)}`));
							});
							req.on("error", (e) => resolve(`error: ${e.message}`));
							req.on("timeout", () => {
								req.destroy();
								resolve("timeout");
							});
						});
					} catch (he) {
						healthOk = `error: ${he.message?.slice(0, 100)}`;
					}
					// Also try fetching the SPA page directly to see if server responds
					var pageHttpOk = "unknown";
					try {
						pageHttpOk = await new Promise((resolve) => {
							var req = http.get(`${baseURL}${path}`, { timeout: 3000 }, (res) => {
								resolve(`${res.statusCode} content-length=${res.headers["content-length"] || "?"}`);
								res.resume();
							});
							req.on("error", (e) => resolve(`error: ${e.message}`));
							req.on("timeout", () => {
								req.destroy();
								resolve("timeout");
							});
						});
					} catch (pe) {
						pageHttpOk = `exception: ${pe.message?.slice(0, 100)}`;
					}
					var diag = [
						`navigateAndWait failed for ${path} after ${attempt + 1} attempts`,
						`page.url(): ${page.url()}`,
						`health: ${healthOk}`,
						`page-http: ${pageHttpOk}`,
						`responses: ${JSON.stringify(responses.slice(0, 5))}`,
						`console: ${JSON.stringify(consoleMessages.slice(0, 10))}`,
						`error: ${error.message?.slice(0, 200)}`,
					].join("\n");
					if (testInfo) {
						await testInfo.attach("navigation-debug", {
							body: Buffer.from(diag, "utf-8"),
							contentType: "text/plain",
						});
						var screenshot = await page.screenshot({ fullPage: true }).catch(() => null);
						if (screenshot) {
							await testInfo.attach("navigation-failure-screenshot", {
								body: screenshot,
								contentType: "image/png",
							});
						}
					}
				} catch {
					// diagnostic collection failed
				}
				break;
			}
		}
	}
	if (lastError) throw lastError;
	return pageErrors;
}

/**
 * Create a new session by clicking the new-session button.
 * Waits for the active key to change, URL to update, and content to mount.
 *
 * Note: we intentionally do NOT wait for the session to appear in the
 * sessions list (store.getByKey). The list is populated asynchronously
 * by the sessions.switch RPC and can be slow under heavy test load.
 * The key change + URL match + page mount are sufficient to prove the
 * session was created; individual tests can wait for store indexing
 * if their assertions require it.
 */
async function createSession(page) {
	const timeoutMs = 10_000;
	const previousActiveKey = await page.evaluate(() => {
		return window.__moltis_stores?.sessionStore?.activeSessionKey?.value || "";
	});

	await page.locator("#newSessionBtn").click();
	await expect
		.poll(
			() =>
				page.evaluate(() => {
					return window.__moltis_stores?.sessionStore?.activeSessionKey?.value || "";
				}),
			{ timeout: timeoutMs },
		)
		.not.toBe(previousActiveKey);

	await expect
		.poll(
			() =>
				page.evaluate(() => {
					const key = window.__moltis_stores?.sessionStore?.activeSessionKey?.value || "";
					if (!key) return false;
					return window.location.pathname === `/chats/${key.replace(/:/g, "/")}`;
				}),
			{ timeout: timeoutMs },
		)
		.toBe(true);

	await expectPageContentMounted(page);
	await waitForChatSessionReady(page);
}

async function waitForChatSessionReady(page) {
	await page.waitForFunction(
		async () => {
			var store = window.__moltis_stores?.sessionStore;
			if (store?.switchInProgress?.value) return false;
			if (document.getElementById("sessionLoadIndicator")) return false;
			var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			if (!appScript) return false;
			var appUrl = new URL(appScript.src, window.location.origin);
			var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			var state = await import(`${prefix}js/state.js`);
			return state.subscribed && !(state.sessionSwitchInProgress || state.chatBatchLoading);
		},
		{ timeout: 5_000 },
	);
}

function isRetryableRpcError(message) {
	if (typeof message !== "string") return false;
	return message.includes("WebSocket not connected") || message.includes("WebSocket disconnected");
}

/**
 * Send an RPC from the page context with retry logic for transient WS errors.
 * Retries a few transient WS disconnects with WS state diagnostics on failure.
 */
async function sendRpcFromPage(page, method, params) {
	let lastResponse = null;
	for (let attempt = 0; attempt < 3; attempt++) {
		if (attempt > 0) {
			const wsState = await page
				.evaluate(() => {
					var s = window.__moltis_state;
					return {
						connected: s?.connected,
						subscribed: s?.subscribed,
						wsExists: !!s?.ws,
						readyState: s?.ws?.readyState,
						pendingCount: s?.pending ? Object.keys(s.pending).length : -1,
					};
				})
				.catch(() => ({}));
			console.log(
				`[sendRpc] ${method} retry #${attempt} ws=${JSON.stringify(wsState)} err=${lastResponse?.error?.message?.slice(0, 60)}`,
			);
			await waitForWsConnected(page, 1_000).catch(() => undefined);
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
	console.log(`[sendRpc] ${method} FAILED after 3 attempts, last: ${lastResponse?.error?.message?.slice(0, 100)}`);
	return lastResponse;
}

async function expectRpcOk(page, method, params) {
	const response = await sendRpcFromPage(page, method, params);
	expect(response?.ok, `RPC ${method} failed: ${response?.error?.message || "unknown error"}`).toBeTruthy();
	return response;
}

module.exports = {
	expectPageContentMounted,
	watchPageErrors,
	waitForWsConnected,
	waitForChatSessionReady,
	navigateAndWait,
	createSession,
	sendRpcFromPage,
	expectRpcOk,
};
