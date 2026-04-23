const { expect, test } = require("../base-test");
const { navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

/**
 * Wait for the chat session to finish loading so injected DOM elements
 * aren't blown away by a late renderHistory() call.
 */
async function waitForSessionReady(page) {
	await page.waitForFunction(async () => {
		var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
		if (!appScript) return false;
		var appUrl = new URL(appScript.src, window.location.origin);
		var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
		var state = await import(`${prefix}js/state.js`);
		return !(state.sessionSwitchInProgress || state.chatBatchLoading);
	}, { timeout: 10_000 });
}

/**
 * Resolve the Vite module prefix from the running page.
 */
async function getModulePrefix(page) {
	return await page.evaluate(() => {
		var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
		if (!appScript) throw new Error("app module script not found");
		var appUrl = new URL(appScript.src, window.location.origin);
		return appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
	});
}

/**
 * Populate the chat message box with enough tall messages to make it scrollable.
 */
async function injectScrollableMessages(page, count) {
	const prefix = await getModulePrefix(page);
	await page.evaluate(({ prefix, msgCount }) => {
		var box = document.getElementById("messages");
		if (!box) throw new Error("chatMsgBox not mounted");
		for (var i = 0; i < msgCount; i++) {
			var el = document.createElement("div");
			el.className = "msg assistant";
			el.textContent = `Message ${i + 1}`;
			el.style.minHeight = "80px";
			box.appendChild(el);
		}
		box.scrollTop = box.scrollHeight;
	}, { prefix, msgCount: count });
}

/**
 * Read the current scroll state from the messages container.
 */
async function getScrollState(page) {
	return await page.evaluate(() => {
		var box = document.getElementById("messages");
		if (!box) return { scrollTop: 0, scrollHeight: 0, clientHeight: 0 };
		return { scrollTop: box.scrollTop, scrollHeight: box.scrollHeight, clientHeight: box.clientHeight };
	});
}

test.describe("Smart auto-scroll", () => {
	test.beforeEach(async ({ page }) => {
		await navigateAndWait(page, "/chats/main");
		await waitForWsConnected(page);
		await waitForSessionReady(page);
	});

	test("new content indicator appears when scrolled up and new message arrives", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await injectScrollableMessages(page, 20);

		// Verify the container is actually scrollable
		const afterFill = await getScrollState(page);
		expect(afterFill.scrollHeight).toBeGreaterThan(afterFill.clientHeight);

		// Scroll to the top to simulate a user reading earlier messages
		await page.evaluate(() => {
			document.getElementById("messages").scrollTop = 0;
		});

		// Add a new assistant message via the smart scroll path
		await page.evaluate(async () => {
			var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			var appUrl = new URL(appScript.src, window.location.origin);
			var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			var chatUi = await import(`${prefix}js/chat-ui.js`);
			var el = chatUi.chatAddMsg("assistant", "New message while scrolled up");
			if (el) el.style.minHeight = "80px";
		});

		// The indicator should be visible
		const indicator = page.locator(".new-content-indicator");
		await expect(indicator).toBeVisible({ timeout: 5_000 });
		await expect(indicator).toHaveText(/New messages/);

		// Scroll position should NOT have jumped back to the bottom
		const afterNewMsg = await getScrollState(page);
		const distanceFromBottom = afterNewMsg.scrollHeight - afterNewMsg.scrollTop - afterNewMsg.clientHeight;
		expect(distanceFromBottom).toBeGreaterThan(60);

		expect(pageErrors).toEqual([]);
	});

	test("clicking indicator scrolls to bottom and hides itself", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await injectScrollableMessages(page, 20);

		// Scroll up, then add a message to trigger the indicator
		await page.evaluate(() => {
			document.getElementById("messages").scrollTop = 0;
		});
		await page.evaluate(async () => {
			var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			var appUrl = new URL(appScript.src, window.location.origin);
			var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			var chatUi = await import(`${prefix}js/chat-ui.js`);
			var el = chatUi.chatAddMsg("assistant", "Trigger indicator");
			if (el) el.style.minHeight = "80px";
		});

		const indicator = page.locator(".new-content-indicator");
		await expect(indicator).toBeVisible({ timeout: 5_000 });

		// Click the indicator
		await indicator.click();

		// Indicator should be gone
		await expect(indicator).toHaveCount(0, { timeout: 5_000 });

		// Verify we are at the bottom
		const afterClick = await getScrollState(page);
		const distanceFromBottom = afterClick.scrollHeight - afterClick.scrollTop - afterClick.clientHeight;
		expect(distanceFromBottom).toBeLessThan(60);

		expect(pageErrors).toEqual([]);
	});

	test("manual scroll to bottom hides indicator", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await injectScrollableMessages(page, 20);

		// Scroll up, add message to trigger indicator
		await page.evaluate(() => {
			document.getElementById("messages").scrollTop = 0;
		});
		await page.evaluate(async () => {
			var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			var appUrl = new URL(appScript.src, window.location.origin);
			var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			var chatUi = await import(`${prefix}js/chat-ui.js`);
			var el = chatUi.chatAddMsg("assistant", "Trigger indicator again");
			if (el) el.style.minHeight = "80px";
		});

		const indicator = page.locator(".new-content-indicator");
		await expect(indicator).toBeVisible({ timeout: 5_000 });

		// Manually scroll to the bottom (simulates user scroll gesture)
		await page.evaluate(() => {
			var box = document.getElementById("messages");
			box.scrollTop = box.scrollHeight;
		});

		// The scroll event listener should have hidden the indicator
		await expect(indicator).toHaveCount(0, { timeout: 5_000 });

		expect(pageErrors).toEqual([]);
	});

	test("user messages always scroll to bottom regardless of scroll position", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await injectScrollableMessages(page, 20);

		// Scroll up
		await page.evaluate(() => {
			document.getElementById("messages").scrollTop = 0;
		});

		// Add a user message — should always scroll to bottom
		await page.evaluate(async () => {
			var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			var appUrl = new URL(appScript.src, window.location.origin);
			var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			var chatUi = await import(`${prefix}js/chat-ui.js`);
			chatUi.chatAddMsg("user", "User message while scrolled up");
		});

		// Should be at the bottom
		const afterUserMsg = await getScrollState(page);
		const distanceFromBottom = afterUserMsg.scrollHeight - afterUserMsg.scrollTop - afterUserMsg.clientHeight;
		expect(distanceFromBottom).toBeLessThan(60);

		// No indicator should have appeared for user messages
		const indicator = page.locator(".new-content-indicator");
		await expect(indicator).toHaveCount(0);

		expect(pageErrors).toEqual([]);
	});

	test('"always" mode bypasses smart scroll and always auto-scrolls', async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		// Set the mode to "always"
		await page.evaluate(async () => {
			var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			var appUrl = new URL(appScript.src, window.location.origin);
			var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			var state = await import(`${prefix}js/state.js`);
			state.setAutoScrollMode("always");
		});

		await injectScrollableMessages(page, 20);

		// Scroll up
		await page.evaluate(() => {
			document.getElementById("messages").scrollTop = 0;
		});

		// Add an assistant message — in "always" mode this should scroll to bottom
		await page.evaluate(async () => {
			var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			var appUrl = new URL(appScript.src, window.location.origin);
			var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			var chatUi = await import(`${prefix}js/chat-ui.js`);
			chatUi.chatAddMsg("assistant", "Message in always mode");
		});

		// Should have scrolled to bottom automatically
		const afterMsg = await getScrollState(page);
		const distanceFromBottom = afterMsg.scrollHeight - afterMsg.scrollTop - afterMsg.clientHeight;
		expect(distanceFromBottom).toBeLessThan(60);

		// No indicator should appear in "always" mode
		const indicator = page.locator(".new-content-indicator");
		await expect(indicator).toHaveCount(0);

		// Reset to default so other tests aren't affected
		await page.evaluate(async () => {
			var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			var appUrl = new URL(appScript.src, window.location.origin);
			var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			var state = await import(`${prefix}js/state.js`);
			state.setAutoScrollMode("smart");
		});

		expect(pageErrors).toEqual([]);
	});
});
