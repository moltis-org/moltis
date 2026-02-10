const { expect } = require("@playwright/test");

/**
 * Wait until the SPA has mounted visible content into #pageContent.
 * This is a stable cross-route readiness signal for the app shell.
 */
async function expectPageContentMounted(page) {
	await expect
		// biome-ignore lint/suspicious/useAwait: page.evaluate returns a Promise
		.poll(async () => {
			return page.evaluate(() => {
				const el = document.getElementById("pageContent");
				if (!el) return 0;
				return el.childElementCount;
			});
		})
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
	await expect(page.locator("#statusDot")).toHaveClass(/connected/, {
		timeout: 15_000,
	});
}

/**
 * Navigate to a path, wait for SPA content to mount, and assert no errors.
 * Returns the pageErrors array for further assertions.
 */
async function navigateAndWait(page, path) {
	const pageErrors = watchPageErrors(page);
	await page.goto(path);
	await expectPageContentMounted(page);
	return pageErrors;
}

/**
 * Create a new session by clicking the new-session button.
 * Waits for URL to change and content to mount.
 */
async function createSession(page) {
	const currentUrl = page.url();
	await page.locator("#newSessionBtn").click();
	await page.waitForURL((url) => url.href !== currentUrl, { timeout: 10_000 });
	await expectPageContentMounted(page);
}

module.exports = {
	expectPageContentMounted,
	watchPageErrors,
	waitForWsConnected,
	navigateAndWait,
	createSession,
};
