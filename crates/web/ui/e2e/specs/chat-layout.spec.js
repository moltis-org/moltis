const { expect, test } = require("../base-test");
const { expectNoPageHorizontalOverflow, navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

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
			"The layout must wrap this text rather than extending the container beyond the viewport. " +
			"very-long-unbroken-message-segment".repeat(16);
		for (var i = 0; i < msgCount; i++) {
			var el = document.createElement("div");
			el.className = i % 2 === 0 ? "msg assistant" : "msg user";
			el.textContent = `[${i + 1}] ${longText}`;
			box.appendChild(el);
		}
	}, count);
}

async function getHorizontalOverflow(page) {
	return await page.evaluate(() => {
		var messages = document.getElementById("messages");
		var composer = document.getElementById("chatComposer");
		var input = document.getElementById("chatInput");
		if (!messages) throw new Error("#messages element not found");
		if (!composer) throw new Error("#chatComposer element not found");
		if (!input) throw new Error("#chatInput element not found");

		var doc = document.documentElement;
		return {
			documentScrollWidth: doc.scrollWidth,
			documentClientWidth: doc.clientWidth,
			messagesScrollWidth: messages.scrollWidth,
			messagesClientWidth: messages.clientWidth,
			composerRight: composer.getBoundingClientRect().right,
			inputScrollWidth: input.scrollWidth,
			inputClientWidth: input.clientWidth,
			viewportWidth: window.innerWidth,
		};
	});
}

async function getChatPaneBounds(page) {
	return await page.evaluate(() => {
		var pageContent = document.getElementById("pageContent");
		if (!pageContent) throw new Error("#pageContent element not found");
		var pageRect = pageContent.getBoundingClientRect();
		return [".chat-toolbar", "#messages", ".chat-input-row", "#chatComposer"].map((selector) => {
			var el = document.querySelector(selector);
			if (!el) throw new Error(`${selector} element not found`);
			var rect = el.getBoundingClientRect();
			return {
				selector,
				left: rect.left,
				right: rect.right,
				pageLeft: pageRect.left,
				pageRight: pageRect.right,
			};
		});
	});
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

	test("long prompt text does not widen the composer or page", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		const longPrompt = `Please inspect this path ${"very-long-unbroken-prompt-segment".repeat(24)} and explain it.`;
		await injectLongMessages(page, 4);

		for (const width of [1119, 600]) {
			await page.setViewportSize({ width, height: 800 });
			await page.waitForFunction((w) => window.innerWidth === w, width, { timeout: 5_000 });

			const chatInput = page.locator("#chatInput");
			await expect(chatInput).toBeVisible({ timeout: 10_000 });
			await chatInput.fill(longPrompt);

			const overflow = await getHorizontalOverflow(page);
			expect(overflow.documentScrollWidth, `document scrollWidth at ${width}px`).toBeLessThanOrEqual(
				overflow.documentClientWidth,
			);
			expect(overflow.messagesScrollWidth, `messages scrollWidth at ${width}px`).toBeLessThanOrEqual(
				overflow.messagesClientWidth,
			);
			expect(overflow.composerRight, `composer right edge at ${width}px`).toBeLessThanOrEqual(overflow.viewportWidth);
			expect(overflow.inputScrollWidth, `input scrollWidth at ${width}px`).toBeLessThanOrEqual(
				overflow.inputClientWidth,
			);
		}

		expect(pageErrors).toEqual([]);
	});

	test("toolbar does not widen chat pane with desktop session sidebar visible (#1055)", async ({ page }, testInfo) => {
		const pageErrors = watchPageErrors(page);

		await page.evaluate(() => {
			var modelLabel = document.getElementById("modelComboLabel");
			if (modelLabel) modelLabel.textContent = "anthropic/claude-sonnet-with-a-very-long-display-name";
			var sandboxImageLabel = document.getElementById("sandboxImageLabel");
			if (sandboxImageLabel) sandboxImageLabel.textContent = "ubuntu:25.10-with-extra-packages";
		});

		for (const width of [1055, 1000, 900, 800]) {
			await page.setViewportSize({ width, height: 880 });
			await page.waitForFunction((w) => window.innerWidth === w, width, { timeout: 5_000 });
			await expect
				.poll(
					() =>
						page.evaluate(() => {
							var panel = document.getElementById("sessionsPanel");
							if (!panel) return false;
							var rect = panel.getBoundingClientRect();
							return rect.width > 200 && getComputedStyle(panel).display !== "none";
						}),
					{ timeout: 5_000 },
				)
				.toBe(true);

			await expectNoPageHorizontalOverflow(page, `chat-sidebar-${width}`, testInfo);

			const bounds = await getChatPaneBounds(page);
			for (const box of bounds) {
				expect(box.left, `${box.selector} left edge at ${width}px`).toBeGreaterThanOrEqual(box.pageLeft - 1);
				expect(box.right, `${box.selector} right edge at ${width}px`).toBeLessThanOrEqual(box.pageRight + 1);
			}
		}

		expect(pageErrors).toEqual([]);
	});
});
