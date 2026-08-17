const { expect, test } = require("../base-test");
const { createSession, navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

/**
 * Wait for the chat session to finish loading AND WS subscribed so injected
 * DOM elements aren't blown away by a late renderHistory() call or reconnect.
 */
async function waitForSessionReady(page) {
	await page.waitForFunction(
		async () => {
			var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			if (!appScript) return false;
			var appUrl = new URL(appScript.src, window.location.origin);
			var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			var state = await import(`${prefix}js/state.js`);
			return state.subscribed && !(state.sessionSwitchInProgress || state.chatBatchLoading);
		},
		{ timeout: 10_000 },
	);
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

async function setAutoScrollMode(page, mode) {
	await page.evaluate(async (nextMode) => {
		var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
		if (!appScript) throw new Error("app module script not found");
		var appUrl = new URL(appScript.src, window.location.origin);
		var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
		var state = await import(`${prefix}js/state.js`);
		state.setAutoScrollMode(nextMode);
	}, mode);
}

async function injectScrollableMessages(page, count) {
	await expect
		.poll(
			() =>
				page.evaluate((msgCount) => {
					var box = document.getElementById("messages");
					if (!box) return 0;

					box.querySelector("#welcomeCard")?.remove();
					box.querySelector("#noProvidersCard")?.remove();
					box.querySelector(".empty-state")?.remove();
					box.classList.remove("chat-messages-empty");

					var fixtures = Array.from(box.querySelectorAll(".msg.assistant[data-e2e-autoscroll-fixture='true']"));
					while (fixtures.length > msgCount) {
						fixtures.pop()?.remove();
					}
					while (fixtures.length < msgCount || box.scrollHeight - box.clientHeight <= 320) {
						var el = document.createElement("div");
						el.className = "msg assistant";
						el.style.flex = "0 0 48px";
						el.style.minHeight = "48px";
						el.dataset.e2eAutoscrollFixture = "true";
						el.textContent = "M".repeat(200);
						box.appendChild(el);
						fixtures.push(el);
						if (fixtures.length > msgCount + 40) break;
					}

					box.querySelector(".new-content-indicator")?.remove();
					box.scrollTop = box.scrollHeight;
					return box.scrollHeight - box.clientHeight;
				}, count),
			{ timeout: 10_000 },
		)
		.toBeGreaterThan(60);
	// Ensure welcome/empty-state cards are gone (they overlap #messages)
	await expect(page.locator("#welcomeCard")).toHaveCount(0, { timeout: 5_000 });
	await expect(page.locator("#messages .empty-state")).toHaveCount(0, { timeout: 2_000 });
	// Scroll to bottom
	await page.evaluate(() => {
		var box = document.getElementById("messages");
		if (box) box.scrollTop = box.scrollHeight;
	});
	await expect
		.poll(async () => {
			const s = await getScrollState(page);
			return s.scrollHeight - s.scrollTop - s.clientHeight;
		})
		.toBeLessThan(60);
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

async function scrollMessagesAwayFromBottom(page) {
	await expect
		.poll(async () => {
			return await page.evaluate(() => {
				var box = document.getElementById("messages");
				if (!box) return 0;

				// Late renders can replace injected fixtures or snap back to bottom in
				// CI. Re-establish overflow and the scrolled-up position in the same
				// browser task that the poll verifies.
				for (let i = 0; i < 20 && box.scrollHeight - box.clientHeight <= 220; i += 1) {
					var el = document.createElement("div");
					el.className = "msg assistant";
					el.style.flex = "0 0 96px";
					el.style.minHeight = "96px";
					el.dataset.e2eAutoscrollFixture = "true";
					el.textContent = "M".repeat(200);
					box.appendChild(el);
				}

				// Stay away from the top edge so this helper does not trigger history
				// autoload, which temporarily disables chatAddMsg() auto-scroll handling.
				box.scrollTop = Math.max(0, box.scrollHeight - box.clientHeight - 200);
				return box.scrollHeight - box.scrollTop - box.clientHeight;
			});
		})
		.toBeGreaterThan(60);
}

async function addAssistantMessageWhileScrolledUp(page, text, minHeight = 80) {
	const result = await page.evaluate(
		async ({ messageText, messageMinHeight }) => {
			var box = document.getElementById("messages");
			if (!box) throw new Error("messages container not found");

			// Recreate enough overflow in the same browser task that triggers
			// smart-scroll. CI can otherwise run a late render between helper calls,
			// leaving the chat close enough to bottom that no indicator is expected.
			for (let i = 0; i < 30 && box.scrollHeight - box.clientHeight <= 320; i += 1) {
				var fixture = document.createElement("div");
				fixture.className = "msg assistant";
				fixture.style.flex = "0 0 96px";
				fixture.style.minHeight = "96px";
				fixture.dataset.e2eAutoscrollFixture = "true";
				fixture.textContent = "M".repeat(200);
				box.appendChild(fixture);
			}

			box.scrollTop = Math.max(0, box.scrollHeight - box.clientHeight - 240);
			const distanceBefore = box.scrollHeight - box.scrollTop - box.clientHeight;
			if (distanceBefore <= 60) {
				throw new Error(`chat was not scrolled away from bottom before message: ${distanceBefore}`);
			}

			var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			var appUrl = new URL(appScript.src, window.location.origin);
			var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			var chatUi = await import(`${prefix}js/chat-ui.js`);
			var el = chatUi.chatAddMsg("assistant", messageText);
			if (el) el.style.minHeight = `${messageMinHeight}px`;

			return {
				distanceBefore,
			};
		},
		{ messageText: text, messageMinHeight: minHeight },
	);
	expect(result.distanceBefore).toBeGreaterThan(60);
}

test.describe("Smart auto-scroll", () => {
	test.beforeEach(async ({ page }, testInfo) => {
		testInfo.setTimeout(90_000);
		await navigateAndWait(page, "/chats/main");
		await waitForWsConnected(page);
		await waitForSessionReady(page);
		// Create a fresh session so no prior history can re-render and
		// overwrite injected DOM elements during the test.
		await createSession(page);
		await waitForSessionReady(page);
		await setAutoScrollMode(page, "smart");
		// Extra settle time for CI — the session switch may trigger
		// deferred renders that overwrite injected DOM.
		await page.waitForTimeout(500);
	});

	test("new content indicator appears when scrolled up and new message arrives", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await injectScrollableMessages(page, 40);

		// Verify the container is actually scrollable
		const afterFill = await getScrollState(page);
		expect(afterFill.scrollHeight).toBeGreaterThan(afterFill.clientHeight);

		// Scroll to the top to simulate a user reading earlier messages
		await scrollMessagesAwayFromBottom(page);

		// Add a new assistant message via the smart scroll path
		await addAssistantMessageWhileScrolledUp(page, "New message while scrolled up");

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
		await injectScrollableMessages(page, 40);

		// Scroll up, then add a message to trigger the indicator
		await scrollMessagesAwayFromBottom(page);
		await addAssistantMessageWhileScrolledUp(page, "Trigger indicator");

		const indicator = page.locator(".new-content-indicator");
		await expect(indicator).toBeVisible({ timeout: 10_000 });

		// Click the current indicator node directly. Playwright's actionability
		// checks can lose a race with the indicator being replaced during scroll
		// recalculation in CI.
		await expect
			.poll(async () => {
				return await page.evaluate(() => {
					var current = document.querySelector(".new-content-indicator");
					if (!current) return true;
					current.click();
					return !document.querySelector(".new-content-indicator");
				});
			})
			.toBe(true);

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
		await injectScrollableMessages(page, 40);

		// Scroll up, add message to trigger indicator
		await scrollMessagesAwayFromBottom(page);
		await addAssistantMessageWhileScrolledUp(page, "Trigger indicator again");

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
		await injectScrollableMessages(page, 40);

		// Scroll up
		await scrollMessagesAwayFromBottom(page);

		// Add a user message — should always scroll to bottom
		await page.evaluate(async () => {
			var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			var appUrl = new URL(appScript.src, window.location.origin);
			var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			var chatUi = await import(`${prefix}js/chat-ui.js`);
			chatUi.chatAddMsg("user", "User message while scrolled up");
			// Wait for rAF-based scroll to complete
			await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
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

	test("auto-scrolls when already at the bottom and new assistant message arrives", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await injectScrollableMessages(page, 40);

		// Verify we are at the bottom after injection (injectScrollableMessages scrolls to end)
		const before = await getScrollState(page);
		const distBefore = before.scrollHeight - before.scrollTop - before.clientHeight;
		expect(distBefore).toBeLessThan(60);

		// Add a new assistant message — should auto-scroll since we're at the bottom
		await page.evaluate(async () => {
			var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			var appUrl = new URL(appScript.src, window.location.origin);
			var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			var chatUi = await import(`${prefix}js/chat-ui.js`);
			var el = chatUi.chatAddMsg("assistant", "New response while at bottom");
			if (el) el.style.minHeight = "80px";
			// Wait for rAF-based scroll to complete
			await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
		});

		// Wait for smooth scroll to finish, then verify at bottom
		await expect
			.poll(async () => {
				const s = await getScrollState(page);
				return s.scrollHeight - s.scrollTop - s.clientHeight;
			})
			.toBeLessThan(60);

		// No "new messages" indicator should appear
		const indicator = page.locator(".new-content-indicator");
		await expect(indicator).toHaveCount(0);

		expect(pageErrors).toEqual([]);
	});

	test("auto-scrolls through multiple sequential assistant messages when at bottom", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await injectScrollableMessages(page, 40);

		// Confirm at bottom
		const before = await getScrollState(page);
		expect(before.scrollHeight - before.scrollTop - before.clientHeight).toBeLessThan(60);

		// Simulate streaming: add several messages one at a time (matching real WS
		// event delivery where each chunk arrives in a separate event loop turn).
		const prefix = await getModulePrefix(page);
		for (let i = 0; i < 5; i++) {
			await page.evaluate(
				async ({ pfx, idx }) => {
					var chatUi = await import(`${pfx}js/chat-ui.js`);
					chatUi.chatAddMsg("assistant", `Streaming chunk ${idx}`);
				},
				{ pfx: prefix, idx: i },
			);
			// Let the rAF-based scroll from smartScrollToBottom complete
			// before adding the next message.
			await page.waitForTimeout(100);
		}

		// Wait for final scroll to settle
		await expect
			.poll(async () => {
				const s = await getScrollState(page);
				return s.scrollHeight - s.scrollTop - s.clientHeight;
			})
			.toBeLessThan(60);

		// No indicator
		const indicator = page.locator(".new-content-indicator");
		await expect(indicator).toHaveCount(0);

		expect(pageErrors).toEqual([]);
	});

	test("auto-scrolls after user message followed by immediate assistant response", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await injectScrollableMessages(page, 40);

		// Confirm at bottom
		const before = await getScrollState(page);
		expect(before.scrollHeight - before.scrollTop - before.clientHeight).toBeLessThan(60);

		// Simulate the exact #946 scenario: user sends, then assistant responds immediately
		await page.evaluate(async () => {
			var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			var appUrl = new URL(appScript.src, window.location.origin);
			var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			var chatUi = await import(`${prefix}js/chat-ui.js`);

			// User message (triggers force scroll)
			var userEl = chatUi.chatAddMsg("user", "Hello, how are you?");
			if (userEl) userEl.style.minHeight = "40px";

			// Immediately after: assistant response starts (like the thinking placeholder)
			var assistantEl = chatUi.chatAddMsg("assistant", "Let me think...");
			if (assistantEl) assistantEl.style.minHeight = "80px";

			// Wait for rAF-based scroll
			await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
		});

		// Wait for smooth scroll to finish, then verify at bottom
		await expect
			.poll(async () => {
				const s = await getScrollState(page);
				return s.scrollHeight - s.scrollTop - s.clientHeight;
			})
			.toBeLessThan(60);

		const indicator = page.locator(".new-content-indicator");
		await expect(indicator).toHaveCount(0);

		expect(pageErrors).toEqual([]);
	});

	test("auto-scrolls when assistant message arrives one frame after user message", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await injectScrollableMessages(page, 40);

		// Confirm at bottom
		const before = await getScrollState(page);
		expect(before.scrollHeight - before.scrollTop - before.clientHeight).toBeLessThan(60);

		// User message scrolls to bottom, then after one rAF the assistant response arrives
		await page.evaluate(async () => {
			var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			var appUrl = new URL(appScript.src, window.location.origin);
			var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			var chatUi = await import(`${prefix}js/chat-ui.js`);

			// User message (force scrolls)
			var userEl = chatUi.chatAddMsg("user", "Question?");
			if (userEl) userEl.style.minHeight = "40px";

			// Wait one rAF so the user-message scroll completes
			await new Promise((resolve) => requestAnimationFrame(resolve));

			// Now assistant message arrives in the next frame
			var assistantEl = chatUi.chatAddMsg("assistant", "Here is the answer...");
			if (assistantEl) assistantEl.style.minHeight = "120px";

			// Wait for the assistant-triggered rAF scroll
			await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
		});

		// Should be at the bottom — the assistant message must have auto-scrolled
		const after = await getScrollState(page);
		expect(after.scrollHeight - after.scrollTop - after.clientHeight).toBeLessThan(60);

		// No indicator
		const indicator = page.locator(".new-content-indicator");
		await expect(indicator).toHaveCount(0);

		expect(pageErrors).toEqual([]);
	});

	test("no indicator appears when at bottom and smartScrollToBottom is called directly", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await injectScrollableMessages(page, 40);

		// Confirm at bottom
		const before = await getScrollState(page);
		expect(before.scrollHeight - before.scrollTop - before.clientHeight).toBeLessThan(60);

		// Call smartScrollToBottom directly (as streaming handlers do)
		await page.evaluate(async () => {
			var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			var appUrl = new URL(appScript.src, window.location.origin);
			var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			var chatUi = await import(`${prefix}js/chat-ui.js`);

			// Append content that pushes scroll down
			var el = document.createElement("div");
			el.className = "msg assistant";
			el.textContent = "Streamed content";
			el.style.minHeight = "40px";
			document.getElementById("messages").appendChild(el);

			// Now call smartScrollToBottom as the WS handler would
			chatUi.smartScrollToBottom();
			await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
		});

		// Wait for smooth scroll to finish, then verify at bottom
		await expect
			.poll(async () => {
				const s = await getScrollState(page);
				return s.scrollHeight - s.scrollTop - s.clientHeight;
			})
			.toBeLessThan(60);

		// No indicator
		const indicator = page.locator(".new-content-indicator");
		await expect(indicator).toHaveCount(0);

		expect(pageErrors).toEqual([]);
	});

	test("rapid message burst while at bottom stays scrolled without indicator", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await injectScrollableMessages(page, 40);

		// Confirm at bottom
		const before = await getScrollState(page);
		expect(before.scrollHeight - before.scrollTop - before.clientHeight).toBeLessThan(60);

		// Fire a rapid burst of messages without waiting between them
		await page.evaluate(async () => {
			var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			var appUrl = new URL(appScript.src, window.location.origin);
			var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			var chatUi = await import(`${prefix}js/chat-ui.js`);

			// Rapid burst — no awaiting between messages (simulates fast streaming)
			for (var i = 0; i < 10; i++) {
				var el = chatUi.chatAddMsg("assistant", `Rapid burst ${i}`);
				if (el) el.style.minHeight = "40px";
			}

			// Wait for rAF scroll to settle
			await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
			// Double rAF for safety — isAutoScrolling guard may defer one frame
			await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
		});

		// Wait for smooth scroll to finish, then verify at bottom
		await expect
			.poll(async () => {
				const s = await getScrollState(page);
				return s.scrollHeight - s.scrollTop - s.clientHeight;
			})
			.toBeLessThan(60);

		// No indicator
		const indicator = page.locator(".new-content-indicator");
		await expect(indicator).toHaveCount(0);

		expect(pageErrors).toEqual([]);
	});

	test('"always" mode bypasses smart scroll and always auto-scrolls', async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		// Set the mode to "always"
		await setAutoScrollMode(page, "always");

		await injectScrollableMessages(page, 40);

		// Scroll up
		await scrollMessagesAwayFromBottom(page);

		// Add an assistant message — in "always" mode this should scroll to bottom
		await page.evaluate(async () => {
			var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			var appUrl = new URL(appScript.src, window.location.origin);
			var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			var chatUi = await import(`${prefix}js/chat-ui.js`);
			chatUi.chatAddMsg("assistant", "Message in always mode");
			// Wait for rAF-based scroll to complete
			await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
		});

		// Wait for smooth scroll to finish, then verify at bottom
		await expect
			.poll(async () => {
				const s = await getScrollState(page);
				return s.scrollHeight - s.scrollTop - s.clientHeight;
			})
			.toBeLessThan(60);

		// No indicator should appear in "always" mode
		const indicator = page.locator(".new-content-indicator");
		await expect(indicator).toHaveCount(0);

		// Reset to default so other tests aren't affected
		await setAutoScrollMode(page, "smart");

		expect(pageErrors).toEqual([]);
	});
});
