const { expect, test } = require("../base-test");
const {
	expectPageContentMounted,
	navigateAndWait,
	openChatMoreModal,
	closeChatMoreModal,
	waitForWsConnected,
	createSession,
	watchPageErrors,
} = require("../helpers");

function isRetryableRpcError(message) {
	if (typeof message !== "string") return false;
	return message.includes("WebSocket not connected") || message.includes("WebSocket disconnected");
}

async function sendRpcFromPage(page, method, params) {
	let lastResponse = null;
	for (let attempt = 0; attempt < 40; attempt++) {
		if (attempt > 0) {
			await waitForWsConnected(page);
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

async function setSwitchRpcSendMode(page, mode, delayMs = 0) {
	await page.evaluate(
		async ({ desiredMode, desiredDelayMs }) => {
			const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			if (!appScript) throw new Error("app module script not found");
			const appUrl = new URL(appScript.src, window.location.origin);
			const prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			const stateModule = await import(`${prefix}js/state.js`);
			const ws = stateModule.ws;
			if (!ws) throw new Error("websocket unavailable");

			if (!window.__origSwitchWsSend) {
				window.__origSwitchWsSend = ws.send.bind(ws);
			}
			if (desiredMode === "restore") {
				ws.send = window.__origSwitchWsSend;
				return;
			}

			ws.send = (payload) => {
				try {
					const parsed = JSON.parse(payload);
					if (parsed?.method === "sessions.switch") {
						if (desiredMode === "drop") return;
						if (desiredMode === "delay") {
							setTimeout(() => window.__origSwitchWsSend(payload), desiredDelayMs);
							return;
						}
					}
				} catch (_err) {
					// Fall through to the original sender.
				}
				return window.__origSwitchWsSend(payload);
			};
		},
		{ desiredMode: mode, desiredDelayMs: delayMs },
	);
}

test.describe("Session management", () => {
	test("session list renders on load", async ({ page }) => {
		await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		const sessionList = page.locator("#sessionList");
		await expect(sessionList).toBeVisible();

		// At least the default "main" session should be present
		const items = sessionList.locator(".session-item");
		await expect(items).not.toHaveCount(0);
	});

	test("sessions sidebar uses search and add button row", async ({ page }) => {
		const pageErrors = await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		const sessionsPanel = page.locator("#sessionsPanel");
		await expect(sessionsPanel).toBeVisible();
		await expect(page.locator("#sessionSearch")).toBeVisible();
		await expect(page.locator("#newSessionBtn")).toBeVisible();

		const hasTopSessionsTitle = await page.evaluate(() => {
			const panel = document.getElementById("sessionsPanel");
			if (!panel) return false;
			const firstBlock = panel.firstElementChild;
			const title = firstBlock?.querySelector("span");
			return (title?.textContent || "").trim() === "Sessions";
		});
		expect(hasTopSessionsTitle).toBe(false);

		const searchAndAddShareRow = await page.evaluate(() => {
			const searchInput = document.getElementById("sessionSearch");
			const newSessionBtn = document.getElementById("newSessionBtn");
			if (!(searchInput && newSessionBtn)) return false;
			return searchInput.parentElement === newSessionBtn.parentElement;
		});
		expect(searchAndAddShareRow).toBe(true);

		expect(pageErrors).toEqual([]);
	});

	test("more controls modal opens and closes on backdrop click", async ({ page }) => {
		const pageErrors = await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		await openChatMoreModal(page);
		await closeChatMoreModal(page);

		expect(pageErrors).toEqual([]);
	});

	test("opening full context from more controls closes the more-controls modal", async ({ page }) => {
		const pageErrors = await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		await openChatMoreModal(page);
		await page.locator("#fullContextBtn").click();
		await expect(page.locator("#chatMoreModal")).toBeHidden({ timeout: 10_000 });
		await expect(page.locator("#fullContextModal")).toBeVisible({ timeout: 10_000 });
		await page.locator("#fullContextModalCloseBtn").click();
		await expect(page.locator("#fullContextModal")).toBeHidden({ timeout: 10_000 });

		expect(pageErrors).toEqual([]);
	});

	test("new session button creates a session", async ({ page }) => {
		const pageErrors = await navigateAndWait(page, "/");
		await waitForWsConnected(page);
		const sessionItems = page.locator("#sessionList .session-item");
		// Wait for the session list to populate via RPC before capturing count
		await expect(sessionItems.first()).toBeVisible();
		const initialCount = await sessionItems.count();

		await createSession(page);
		const firstSessionPath = new URL(page.url()).pathname;
		const firstSessionKey = firstSessionPath.replace(/^\/chats\//, "").replace(/\//g, ":");

		// URL should change to a new session (not main)
		await expect(page).not.toHaveURL(/\/chats\/main$/);
		await expect(page).toHaveURL(/\/chats\//);
		await expect(page.locator(`#sessionList .session-item[data-session-key="${firstSessionKey}"]`)).toHaveClass(
			/active/,
		);
		await expect(sessionItems).toHaveCount(initialCount + 1);
		await expect(page.locator("#chatInput")).toBeFocused();

		// Regression: creating a second session should still update the list
		// and mark the new session as active.
		await createSession(page);
		const secondSessionPath = new URL(page.url()).pathname;
		const secondSessionKey = secondSessionPath.replace(/^\/chats\//, "").replace(/\//g, ":");
		await expect(page.locator(`#sessionList .session-item[data-session-key="${secondSessionKey}"]`)).toHaveClass(
			/active/,
		);
		await expect(sessionItems).toHaveCount(initialCount + 2);
		await expect(page.locator("#chatInput")).toBeFocused();

		expect(pageErrors).toEqual([]);
	});

	test("clicking a session switches to it", async ({ page }) => {
		await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		// Create a second session so we have something to switch to
		await createSession(page);
		const newSessionUrl = page.url();

		// Click the "main" session in the list
		const mainItem = page.locator('#sessionList .session-item[data-session-key="main"]');
		// If data-session-key isn't set, fall back to finding by label text
		const target = (await mainItem.count()) ? mainItem : page.locator("#sessionList .session-item").first();
		await target.click();

		await expect(page).not.toHaveURL(newSessionUrl);
		await expectPageContentMounted(page);
	});

	test("modifier-clicking a session opens it in a new tab", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		await createSession(page);
		const currentUrl = page.url();

		const mainItem = page.locator('#sessionList .session-item[data-session-key="main"]');
		await expect(mainItem).toBeVisible({ timeout: 5_000 });

		const newPagePromise = new Promise((resolve) => {
			page.context().once("page", (openedPage) => {
				resolve({
					newPage: openedPage,
					newPageErrors: watchPageErrors(openedPage),
				});
			});
		});
		await mainItem.click({
			modifiers: [process.platform === "darwin" ? "Meta" : "Control"],
		});
		const { newPage, newPageErrors } = await newPagePromise;

		await newPage.waitForLoadState("domcontentloaded");
		await expectPageContentMounted(newPage);
		await waitForWsConnected(newPage);
		await expect(newPage).toHaveURL(/\/chats\/main$/);
		await expect(page).toHaveURL(currentUrl);

		expect(pageErrors).toEqual([]);
		expect(newPageErrors).toEqual([]);
		await newPage.close();
	});

	test("shows loading indicator while uncached session switch is pending", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		await setSwitchRpcSendMode(page, "drop");
		await page.locator("#newSessionBtn").click();
		await expect(page.locator("#sessionLoadIndicator")).toBeVisible();
		await setSwitchRpcSendMode(page, "restore");

		expect(pageErrors).toEqual([]);
	});

	test("cached session history renders instantly while switch refreshes in background", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		await createSession(page);
		const sessionPath = new URL(page.url()).pathname;
		const sessionKey = sessionPath.replace(/^\/chats\//, "").replace(/\//g, ":");

		const cachedText = "cached history should appear instantly";
		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey,
				state: "final",
				text: cachedText,
				messageIndex: 0,
				model: "test-model",
				provider: "test-provider",
				replyMedium: "text",
				runId: "run-cached-session",
			},
		});
		await expect(page.locator("#messages .msg.assistant").filter({ hasText: cachedText })).toBeVisible();

		await page.locator('#sessionList .session-item[data-session-key="main"]').click();
		await expect(page).toHaveURL(/\/chats\/main$/);

		await setSwitchRpcSendMode(page, "delay", 900);
		await page.locator(`#sessionList .session-item[data-session-key="${sessionKey}"]`).click();
		await expect(page.locator("#messages .msg.assistant").filter({ hasText: cachedText })).toBeVisible({
			timeout: 300,
		});
		await expect(page.locator("#sessionLoadIndicator")).toHaveCount(0);
		await setSwitchRpcSendMode(page, "restore");

		expect(pageErrors).toEqual([]);
	});

	test("main session hides delete while non-main sessions show delete in more controls", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/");
		await waitForWsConnected(page);
		await expectPageContentMounted(page);

		await openChatMoreModal(page);
		await expect(page.locator('#chatMoreModal button[title="Clear session"]')).toBeHidden();
		await expect(page.locator('#chatMoreModal button[title="Delete session"]')).toHaveCount(0);
		await closeChatMoreModal(page);

		await createSession(page);

		await openChatMoreModal(page);
		await expect(page.locator('#chatMoreModal button[title="Clear session"]')).toHaveCount(0);
		await expect(page.locator('#chatMoreModal button[title="Delete session"]')).toBeVisible();
		await closeChatMoreModal(page);

		expect(pageErrors).toEqual([]);
	});

	test("stop action appears for active run and clears after abort", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/");
		await waitForWsConnected(page);
		await expectPageContentMounted(page);
		await createSession(page);

		const sessionPath = new URL(page.url()).pathname;
		const sessionKey = sessionPath.replace(/^\/chats\//, "").replace(/\//g, ":");

		const stopBtn = page.locator('#sessionHeaderToolbarMount button[title="Stop generation"]');
		await expect(stopBtn).toHaveCount(0);
		await openChatMoreModal(page);
		await expect(page.locator('#chatMoreModal button[title="Delete session"]')).toBeVisible();
		await closeChatMoreModal(page);

		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey,
				state: "thinking",
				runId: "run-stop-e2e",
			},
		});

		await expect(stopBtn).toBeVisible();
		await stopBtn.click();
		await expect(stopBtn).toHaveCount(0);
		await openChatMoreModal(page);
		await expect(page.locator('#chatMoreModal button[title="Delete session"]')).toBeVisible();
		await closeChatMoreModal(page);

		expect(pageErrors).toEqual([]);
	});

	test("share button creates cutoff notice and copyable link", async ({ page }) => {
		const pageErrors = await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		await page.evaluate(() => {
			window.__shareTestCopiedLink = "";
			window.__shareTestPromptLink = "";
			window.prompt = (_message, defaultValue) => {
				window.__shareTestPromptLink = typeof defaultValue === "string" ? defaultValue : "";
				return window.__shareTestPromptLink;
			};
			try {
				Object.defineProperty(window.navigator, "clipboard", {
					configurable: true,
					value: {
						writeText: (value) => {
							window.__shareTestCopiedLink = String(value);
						},
					},
				});
			} catch (_err) {
				// Browser may expose clipboard as non-configurable in tests.
			}
		});

		await openChatMoreModal(page);
		await page.locator('#chatMoreModal button[title="Share snapshot"]').click();
		await expect(page.locator("#chatMoreModal")).toBeHidden({ timeout: 10_000 });
		await expect(page.locator('[data-share-visibility="public"]')).toBeVisible({ timeout: 10_000 });
		await expect(
			page.getByText(
				"We do best-effort redaction for API keys and tokens in shared tool output, but always review before sharing.",
			),
		).toBeVisible();
		await page.locator('[data-share-visibility="public"]').click();

		await expect
			.poll(() => page.evaluate(() => window.__shareTestCopiedLink || window.__shareTestPromptLink || ""), {
				timeout: 10_000,
			})
			.toMatch(/\/share\//);

		await expect(
			page.locator(".msg.system").filter({
				hasText: "This session until here has been shared. Later messages are not included in the shared link.",
			}),
		).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("share copy fallback uses styled modal instead of browser prompt", async ({ page }) => {
		await page.addInitScript(() => {
			window.__sharePromptCalled = false;
			window.prompt = () => {
				window.__sharePromptCalled = true;
				return "";
			};
			Object.defineProperty(window.navigator, "clipboard", {
				configurable: true,
				value: {
					writeText: () => Promise.reject(new Error("clipboard blocked for test")),
				},
			});
		});

		const pageErrors = await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		await openChatMoreModal(page);
		await page.locator('#chatMoreModal button[title="Share snapshot"]').click();
		await expect(page.locator("#chatMoreModal")).toBeHidden({ timeout: 10_000 });
		await expect(page.locator('[data-share-visibility="public"]')).toBeVisible({ timeout: 10_000 });
		await page.locator('[data-share-visibility="public"]').click();

		const linkModal = page.locator('[data-share-link-modal="true"]');
		await expect(linkModal).toBeVisible();
		await expect(page.locator('[data-share-link-input="true"]')).toHaveValue(/\/share\//);

		const promptCalled = await page.evaluate(() => window.__sharePromptCalled === true);
		expect(promptCalled).toBe(false);

		await page.locator('[data-share-link-close="true"]').click();
		await expect(linkModal).toHaveCount(0);

		expect(pageErrors).toEqual([]);
	});

	test("private share requires key and strips it from URL", async ({ page, request }) => {
		const pageErrors = await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		const create = await expectRpcOk(page, "sessions.share.create", {
			key: "main",
			visibility: "private",
		});
		const sharePath = create?.payload?.path || "";
		const accessKey = create?.payload?.accessKey || "";
		expect(sharePath).toMatch(/^\/share\/.+/);
		expect(accessKey).toBeTruthy();

		const deniedResponse = await request.get(sharePath);
		expect(deniedResponse.status()).toBe(404);

		await page.goto(`${sharePath}?k=${encodeURIComponent(accessKey)}`);
		await page.waitForURL((url) => url.pathname === sharePath && !url.searchParams.has("k"), { timeout: 10_000 });

		await expect(page.locator("main")).toBeVisible();
		await expect(page.locator("a[href='https://www.moltis.org']")).toBeVisible();
		const shareFooter = page.locator(".share-page-footer");
		await expect(shareFooter).toContainText("Get your AI assistant at");
		await expect(shareFooter.locator("strong")).toHaveCount(0);
		await expect(page.locator("#chatInput")).toHaveCount(0);
		await expect(page.locator("meta[property='og:image']")).toHaveCount(1);
		await expect(page.locator(".theme-toggle")).toBeVisible();
		await expect(page.locator('.theme-btn[data-theme-val="light"]')).toBeVisible();
		await expect(page.locator('.theme-btn[data-theme-val="dark"]')).toBeVisible();
		await expect(page.locator("script[nonce]")).toHaveCount(0);
		await expect(page.locator(".share-time")).toHaveCount(0);
		const imageViewer = page.locator('[data-image-viewer="true"]');
		await expect(imageViewer).toHaveCount(1);
		await expect(imageViewer).toHaveAttribute("aria-hidden", "true");

		expect(pageErrors).toEqual([]);
	});
	test("main session preview updates after clear on first message without reload", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/chats/main");
		await waitForWsConnected(page);

		const chatInput = page.locator("#chatInput");
		await expect(chatInput).toBeVisible();
		await expect(chatInput).toBeEnabled();

		await chatInput.fill("/clear");
		await chatInput.press("Enter");

		await expect
			.poll(
				() =>
					page.evaluate(() => {
						const store = window.__moltis_stores?.sessionStore;
						const main = store?.getByKey?.("main");
						if (!main) return null;
						return {
							messageCount: main.messageCount || 0,
							preview: main.preview || "",
						};
					}),
				{ timeout: 10_000 },
			)
			.toEqual({ messageCount: 0, preview: "" });

		const firstMessage = "sidebar preview should update immediately";
		await chatInput.fill(firstMessage);
		await chatInput.press("Enter");

		await expect(page.locator('#sessionList .session-item[data-session-key="main"] .session-preview')).toContainText(
			firstMessage,
		);

		expect(pageErrors).toEqual([]);
	});
	test("session search filters the list", async ({ page }) => {
		await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		const searchInput = page.locator("#sessionSearch");
		// searchInput may be hidden until focused or may always be visible
		if (await searchInput.isVisible()) {
			// Wait for session list to populate before capturing baseline count
			await expect(page.locator("#sessionList .session-item").first()).toBeVisible({
				timeout: 5_000,
			});
			const countBefore = await page.locator("#sessionList .session-item").count();

			// Type a string that won't match any session
			await searchInput.fill("zzz_no_match_zzz");
			// Allow time for filtering
			await page.waitForTimeout(300);

			const countAfter = await page.locator("#sessionList .session-item").count();
			expect(countAfter).toBeLessThanOrEqual(countBefore);

			// Clear search restores list
			await searchInput.fill("");
			await page.waitForTimeout(300);

			const countRestored = await page.locator("#sessionList .session-item").count();
			expect(countRestored).toBe(countBefore);
		}
	});

	test("clear all sessions resets list", async ({ page }) => {
		await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		// Create extra sessions first
		await createSession(page);
		await createSession(page);

		await openChatMoreModal(page);
		await page.locator("#chatMoreDeleteAllBtn").click();
		await expect(page.locator("#chatMoreModal")).toBeHidden({ timeout: 10_000 });

		const confirmModal = page.locator(".provider-modal-backdrop:not(.hidden)").filter({
			hasText: /Delete \d+ sessions\?/,
		});
		await expect(confirmModal).toBeVisible({ timeout: 10_000 });
		await confirmModal.getByRole("button", { name: "Delete", exact: true }).click();
		await expect(confirmModal).toHaveCount(0, { timeout: 10_000 });

		await expectPageContentMounted(page);
		const items = page.locator("#sessionList .session-item");
		const count = await items.count();
		expect(count).toBeGreaterThanOrEqual(1);
	});

	test("sessions panel hidden on non-chat pages", async ({ page }) => {
		await navigateAndWait(page, "/settings");

		const panel = page.locator("#sessionsPanel");
		// On settings pages, the sessions panel should be hidden
		await expect(panel).toBeHidden();
	});

	test("deleting unmodified fork skips confirmation dialog", async ({ page }) => {
		const pageErrors = await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		// Create a session so we're not on "main" (Delete button is hidden for main)
		await createSession(page);
		const sessionUrl = page.url();
		await openChatMoreModal(page);
		const deleteBtn = page.locator('#chatMoreModal button[title="Delete session"]');
		await expect(deleteBtn).toBeVisible({ timeout: 10_000 });

		// Simulate an unmodified fork before deleting.
		await expect
			.poll(
				() =>
					page.evaluate(() => {
						const store = window.__moltis_stores?.sessionStore;
						const session = store?.activeSession?.value;
						if (!session) return false;
						session.forkPoint = 5;
						session.messageCount = 5;
						session.dataVersion.value++;
						return true;
					}),
				{ timeout: 10_000 },
			)
			.toBe(true);
		await deleteBtn.click();
		await expect(page.locator("#chatMoreModal")).toBeHidden({ timeout: 10_000 });

		// The confirmation dialog should NOT be visible.
		const confirmModal = page.locator(".provider-modal-backdrop:not(.hidden)").filter({
			hasText: "Delete this session?",
		});
		await expect(confirmModal).toHaveCount(0);

		// The session should be deleted immediately (no dialog appeared)
		// so we should navigate away from the current session URL.
		// switchSession uses history.replaceState (no navigation event),
		// so poll the URL rather than using waitForURL which waits for "load".
		await expect.poll(() => page.url(), { timeout: 10_000 }).not.toBe(sessionUrl);

		expect(pageErrors).toEqual([]);
	});

	test("deleting modified fork still shows confirmation dialog", async ({ page }) => {
		const pageErrors = await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		await createSession(page);

		// Simulate a modified fork: messageCount > forkPoint
		await expect
			.poll(
				() =>
					page.evaluate(() => {
						const store = window.__moltis_stores?.sessionStore;
						const session = store?.activeSession?.value;
						if (!session) return false;
						session.forkPoint = 3;
						session.messageCount = 5;
						session.dataVersion.value++;
						return true;
					}),
				{ timeout: 10_000 },
			)
			.toBe(true);

		await openChatMoreModal(page);
		const deleteBtn = page.locator('#chatMoreModal button[title="Delete session"]');
		await expect(deleteBtn).toBeVisible();
		await deleteBtn.click();
		await expect(page.locator("#chatMoreModal")).toBeHidden({ timeout: 10_000 });

		// The confirmation dialog SHOULD appear
		const confirmModal = page.locator(".provider-modal-backdrop:not(.hidden)").filter({
			hasText: "Delete this session?",
		});
		await expect(confirmModal).toBeVisible({ timeout: 10_000 });

		// Dismiss the dialog by clicking Cancel
		await confirmModal.getByRole("button", { name: "Cancel", exact: true }).click();
		await expect(confirmModal).toHaveCount(0);

		expect(pageErrors).toEqual([]);
	});

	test("toggling sandbox shows chat notice", async ({ page }) => {
		const pageErrors = await navigateAndWait(page, "/chats/main");
		await waitForWsConnected(page);

		// Enable sandbox via RPC patch
		await expectRpcOk(page, "sessions.patch", {
			key: "main",
			sandboxEnabled: true,
		});

		// The chat notice should appear as a system message
		await expect(page.locator(".msg.system").filter({ hasText: "Sandbox enabled" })).toBeVisible({ timeout: 5_000 });

		// Disable sandbox
		await expectRpcOk(page, "sessions.patch", {
			key: "main",
			sandboxEnabled: false,
		});

		await expect(page.locator(".msg.system").filter({ hasText: "Sandbox disabled" })).toBeVisible({ timeout: 5_000 });

		expect(pageErrors).toEqual([]);
	});

	test("channel-bound session can be renamed", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		// Create a session with a channel-like key (telegram prefix triggers isChannel detection).
		const channelKey = `telegram:bot:rename-test-${Date.now()}`;
		await expectRpcOk(page, "sessions.switch", { key: channelKey });

		// Give the session an initial display name before the rename step.
		await expectRpcOk(page, "sessions.patch", { key: channelKey, label: "Telegram 1" });

		// Channel-bound sessions are listed in the regular Sessions tab.
		const sessionsTab = page.locator('#sessionTabBar .session-tab[data-tab="sessions"]');
		await expect(sessionsTab).toBeVisible({ timeout: 5_000 });
		await sessionsTab.click();

		// Click the channel session to select it.
		const channelItem = page.locator(`#sessionList .session-item[data-session-key="${channelKey}"]`);
		await expect(channelItem).toBeVisible({ timeout: 10_000 });
		await channelItem.click();

		// Open session controls and start rename from the modal.
		await openChatMoreModal(page);
		const renameBtn = page.locator('#chatMoreModal button[title="Rename session"]');
		await expect(renameBtn).toBeVisible({ timeout: 5_000 });
		await renameBtn.click();
		const renameInput = page.locator("#chatMoreModal .chat-session-rename-input");
		await expect(renameInput).toBeVisible({ timeout: 5_000 });

		// Type a new name and press Enter.
		const newName = "My Discord Chat";
		await renameInput.fill(newName);
		await renameInput.press("Enter");

		// Verify the rename stuck in the sidebar.
		await expect(
			page.locator(`#sessionList .session-item[data-session-key="${channelKey}"] [data-label-text]`),
		).toHaveText(newName, { timeout: 5_000 });

		expect(pageErrors).toEqual([]);
	});

	test("cron session shows delete button in more controls", async ({ page }) => {
		const pageErrors = await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		// Create a cron session in the database via sessions.switch, then add a
		// message so messageCount > 0 (triggers the confirmation dialog on delete).
		const cronKey = `cron:e2e-delete-test-${Date.now()}`;
		await expectRpcOk(page, "sessions.switch", { key: cronKey });
		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey: cronKey,
				state: "final",
				text: "cron output",
				messageIndex: 0,
				model: "test-model",
				provider: "test-provider",
				replyMedium: "text",
				runId: "run-cron-delete",
			},
		});

		// Switch to the cron tab so the session is visible
		const cronTab = page.locator('#sessionTabBar .session-tab[data-tab="cron"]');
		await expect(cronTab).toBeVisible({ timeout: 5_000 });
		await cronTab.click();

		// Click the cron session to select it
		const cronItem = page.locator(`#sessionList .session-item[data-session-key="${cronKey}"]`);
		await expect(cronItem).toBeVisible({ timeout: 10_000 });
		await cronItem.click();

		// Wait for session messages to be fully loaded before proceeding
		await expect(page.locator(".msg")).not.toHaveCount(0, { timeout: 5_000 });

		// Open more controls and verify delete button is visible
		await openChatMoreModal(page);
		const deleteBtn = page.locator('#chatMoreModal button[title="Delete session"]');
		await expect(deleteBtn).toBeVisible({ timeout: 5_000 });

		// Click delete — should show confirmation since it has messages
		await deleteBtn.click();
		await expect(page.locator("#chatMoreModal")).toBeHidden({ timeout: 10_000 });

		const confirmModal = page.locator(".provider-modal-backdrop:not(.hidden)").filter({
			hasText: "Delete this session?",
		});
		await expect(confirmModal).toBeVisible({ timeout: 10_000 });
		await confirmModal.getByRole("button", { name: "Delete", exact: true }).click();
		await expect(confirmModal).toHaveCount(0, { timeout: 10_000 });

		// Cron session should be gone from the list
		await expect(cronItem).toHaveCount(0, { timeout: 10_000 });

		expect(pageErrors).toEqual([]);
	});
});
