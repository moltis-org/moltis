const { expect } = require("@playwright/test");

/**
 * Wait until the SPA has mounted visible content into #pageContent.
 * This is a stable cross-route readiness signal for the app shell.
 */
async function expectPageContentMounted(page) {
	await expect
		// biome-ignore lint/suspicious/useAwait: page.evaluate returns a Promise
		.poll(
			async () => {
				return page.evaluate(() => {
					const el = document.getElementById("pageContent");
					if (!el) return 0;
					return el.childElementCount;
				});
			},
			{
				timeout: 20_000,
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
async function waitForWsConnected(page) {
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
					.evaluate(async () => {
						const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
						if (!appScript) return false;
						const appUrl = new URL(appScript.src, window.location.origin);
						const prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
						const state = await import(`${prefix}js/state.js`);
						return Boolean(state.connected && state.ws && state.ws.readyState === WebSocket.OPEN);
					})
					.catch(() => false);
			},
			{ timeout: 20_000 },
		)
		.toBe(true);
}

/**
 * Navigate to a path, wait for SPA content to mount, and assert no errors.
 * Returns the pageErrors array for further assertions.
 */
async function navigateAndWait(page, path) {
	const pageErrors = watchPageErrors(page);
	let lastError = null;
	for (let attempt = 0; attempt < 2; attempt++) {
		await page.goto(path, { waitUntil: "domcontentloaded" });
		try {
			await expectPageContentMounted(page);
			return pageErrors;
		} catch (error) {
			lastError = error;
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
	const timeoutMs = 20_000;
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
}

module.exports = {
	expectPageContentMounted,
	watchPageErrors,
	waitForWsConnected,
	navigateAndWait,
	createSession,
};
