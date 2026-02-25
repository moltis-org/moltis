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
 * Waits for URL to change and content to mount.
 */
async function createSession(page) {
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
			{ timeout: 20_000 },
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
			{ timeout: 20_000 },
		)
		.toBe(true);

	await expectPageContentMounted(page);
	await expect
		.poll(
			() =>
				page.evaluate(() => {
					const store = window.__moltis_stores?.sessionStore;
					if (!store) return false;
					const activeKey = store.activeSessionKey?.value || "";
					if (!activeKey) return false;

					const activeSession = store.getByKey ? store.getByKey(activeKey) : store.activeSession?.value;
					return Boolean(activeSession && activeSession.key === activeKey);
				}),
			{ timeout: 20_000 },
		)
		.toBe(true);
}

module.exports = {
	expectPageContentMounted,
	watchPageErrors,
	waitForWsConnected,
	navigateAndWait,
	createSession,
};
