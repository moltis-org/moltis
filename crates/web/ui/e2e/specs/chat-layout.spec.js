const { expect, test } = require("../base-test");
const { navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

/**
 * Wait for the chat session to finish loading so injected DOM elements
 * aren't blown away by a late renderHistory() call.
 */
async function waitForSessionReady(page) {
	await page.waitForFunction(
		async () => {
			var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			if (!appScript) return false;
			var appUrl = new URL(appScript.src, window.location.origin);
			var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			var state = await import(`${prefix}js/state.js`);
			return !(state.sessionSwitchInProgress || state.chatBatchLoading);
		},
		{ timeout: 10_000 },
	);
}

/**
 * Inject messages with long text content to stress the layout.
 */
async function injectLongMessages(page, count) {
	await page.evaluate((msgCount) => {
		var box = document.getElementById("messages");
		if (!box) throw new Error("#messages element not found");
		var longText =
			"This is a fairly long message that contains enough text to potentially cause horizontal overflow " +
			"if the container does not properly constrain its width. It includes some inline code like " +
			"`const result = await fetch('/api/endpoint')` and continues with more text to fill the line. " +
			"The layout must wrap this text rather than extending the container beyond the viewport.";
		for (var i = 0; i < msgCount; i++) {
			var el = document.createElement("div");
			el.className = i % 2 === 0 ? "msg assistant" : "msg user";
			el.textContent = `[${i + 1}] ${longText}`;
			box.appendChild(el);
		}
	}, count);
}

test.describe("Chat layout — no horizontal overflow (#945)", () => {
	test.beforeEach(async ({ page }) => {
		await navigateAndWait(page, "/chats/main");
		await waitForWsConnected(page);
		await waitForSessionReady(page);
	});

	test("messages container does not scroll horizontally with long content", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await injectLongMessages(page, 10);

		// The messages container must not have horizontal overflow
		const overflow = await page.evaluate(() => {
			var box = document.getElementById("messages");
			if (!box) throw new Error("#messages element not found");
			return { scrollWidth: box.scrollWidth, clientWidth: box.clientWidth };
		});
		expect(overflow.scrollWidth).toBeLessThanOrEqual(overflow.clientWidth);

		expect(pageErrors).toEqual([]);
	});

	test("chat layout fits viewport at various widths", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await injectLongMessages(page, 6);

		for (const width of [1280, 900, 600]) {
			await page.setViewportSize({ width, height: 800 });
			// Allow layout to settle after resize
			await page.waitForFunction((w) => window.innerWidth === w, width, { timeout: 5_000 });

			const overflow = await page.evaluate(() => {
				var box = document.getElementById("messages");
				if (!box) throw new Error("#messages element not found");
				return { scrollWidth: box.scrollWidth, clientWidth: box.clientWidth };
			});

			// No horizontal scrollbar: content fits within the visible area
			expect(overflow.scrollWidth, `scrollWidth <= clientWidth at ${width}px`).toBeLessThanOrEqual(
				overflow.clientWidth,
			);
		}

		expect(pageErrors).toEqual([]);
	});
});
