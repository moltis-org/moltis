const { expect, test } = require("../base-test");
const { navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

function isRetryableMockError(error) {
	const message = error?.message || String(error || "");
	return (
		message.includes("net::ERR_ABORTED") ||
		message.includes("Execution context was destroyed") ||
		message.includes("Target page, context or browser has been closed") ||
		message.includes("Timeout") ||
		message.includes("exceeded while waiting")
	);
}

async function mockChannelsStatus(page, { channels, senders = [], allowRetryOwnership = false, label }) {
	const firstMarker = channels
		.map((channel) => String(channel.name || channel.account_id || channel.details || "").trim())
		.find(Boolean);
	let lastError = null;
	for (let attempt = 0; attempt < 3; attempt++) {
		try {
			await expect.poll(() => new URL(page.url()).pathname).toBe("/settings/channels");
			await expect(page.getByRole("heading", { name: "Channels", exact: true })).toBeVisible();
			await page.waitForFunction(() => !!document.querySelector('script[type="module"][src*="js/app.js"]'));
			await page.evaluate(
				async (fixture) => {
					const {
						channels: mockChannels,
						senders: mockSenders,
						allowRetryOwnership: mockAllowRetryOwnership,
						label: mockLabel,
					} = fixture;
					const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
					if (!appScript) throw new Error("app.js script not found");
					const appUrl = new URL(appScript.src, window.location.origin).href;
					const marker = "js/app.js";
					const markerIdx = appUrl.indexOf(marker);
					if (markerIdx < 0) throw new Error("app.js marker not found in script URL");
					const prefix = appUrl.slice(0, markerIdx);
					const state = await import(`${prefix}js/state.js`);
					const channelsPage = await import(`${prefix}js/page-channels.js`);
					const wsOpen = typeof WebSocket !== "undefined" ? WebSocket.OPEN : 1;
					window.__matrixOwnershipRetryRequest = null;
					const responseFor = (req) => {
						if (req.method === "channels.status") {
							return { ok: true, payload: { channels: mockChannels } };
						}
						if (req.method === "channels.senders.list") {
							return { ok: true, payload: { senders: mockSenders } };
						}
						if (req.method === "channels.retry_ownership" && mockAllowRetryOwnership) {
							window.__matrixOwnershipRetryRequest = req.params;
							return { ok: true, payload: { ok: true } };
						}
						return {
							ok: false,
							error: { message: `unexpected rpc in ${mockLabel}: ${req.method}` },
						};
					};
					state.setWs({
						readyState: wsOpen,
						send(raw) {
							const req = JSON.parse(raw || "{}");
							const resolver = state.pending[req.id];
							if (!resolver) return;
							resolver(responseFor(req));
							delete state.pending[req.id];
						},
					});
					state.setConnected(true);
					if (typeof state.refreshChannelsPage === "function") {
						state.refreshChannelsPage();
					} else {
						await channelsPage.prefetchChannels();
					}
					await new Promise((resolve) => setTimeout(resolve, 100));
				},
				{ channels, senders, allowRetryOwnership, label },
			);
			if (channels.length === 0) {
				await expect(page.getByText("No channels connected.", { exact: false })).toBeVisible();
			} else {
				await expect
					.poll(
						async () => {
							const cardCount = await page
								.locator(".provider-card")
								.count()
								.catch(() => 0);
							const pageText = await page
								.locator("#pageContent")
								.innerText()
								.catch(() => "");
							return cardCount >= channels.length && (!firstMarker || pageText.includes(firstMarker));
						},
						{ timeout: 10_000 },
					)
					.toBe(true);
			}
			return;
		} catch (error) {
			lastError = error;
			if (!isRetryableMockError(error) || attempt === 2) break;
		}
	}
	if (lastError) throw lastError;
}

test.describe("Settings channels", () => {
	test("channels add telegram token field is treated as a password", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const addButton = page.getByRole("button", { name: "Connect Telegram", exact: true });
		await expect(addButton).toBeVisible();
		await addButton.click();

		await expect(page.getByRole("heading", { name: "Connect Telegram", exact: true })).toBeVisible();
		const tokenInput = page.getByPlaceholder("123456:ABC-DEF...");
		await expect(tokenInput).toHaveAttribute("type", "password");
		await expect(tokenInput).toHaveAttribute("autocomplete", "new-password");
		await expect(tokenInput).toHaveAttribute("name", "telegram_bot_token");
		expect(pageErrors).toEqual([]);
	});

	test("channels add matrix supports access token auth and auto-generates an account id", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);
		await expect(page.getByText(/stored in Moltis's internal database \(.+moltis\.db\)/)).toBeVisible();

		const addButton = page.getByRole("button", { name: "Connect Matrix", exact: true });
		await expect(addButton).toBeVisible();
		await addButton.click();

		const modal = page.locator(".modal-box");
		await expect(modal.getByRole("heading", { name: "Connect Matrix", exact: true })).toBeVisible();
		await expect(modal.locator('input[data-field="accountId"]')).toHaveCount(0);
		await expect(modal.locator('input[data-field="homeserver"]')).toHaveValue("https://matrix.org");
		await expect(modal.locator('input[data-field="homeserver"]')).toHaveAttribute("placeholder", "https://matrix.org");
		await expect(modal.locator('select[data-field="authMode"]')).toHaveValue("oidc");
		await expect(
			modal.getByText("Encrypted Matrix chats require OIDC or Password auth.", { exact: false }),
		).toBeVisible();
		await expect(
			modal.getByText("Use OIDC (recommended) or Password so Moltis creates and persists its own Matrix device keys", {
				exact: false,
			}),
		).toBeVisible();
		await expect(modal.getByText("verify yes", { exact: false })).toBeVisible();

		await page.evaluate(async () => {
			const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			if (!appScript) throw new Error("app.js script not found");
			const appUrl = new URL(appScript.src, window.location.origin).href;
			const marker = "js/app.js";
			const markerIdx = appUrl.indexOf(marker);
			if (markerIdx < 0) throw new Error("app.js marker not found in script URL");
			const prefix = appUrl.slice(0, markerIdx);
			const state = await import(`${prefix}js/state.js`);
			const wsOpen = typeof WebSocket !== "undefined" ? WebSocket.OPEN : 1;
			window.__matrixSettingsAddRequest = null;
			state.setConnected(true);
			state.setWs({
				readyState: wsOpen,
				send(raw) {
					const req = JSON.parse(raw || "{}");
					const resolver = state.pending[req.id];
					if (!resolver) return;
					if (req.method === "channels.add") {
						window.__matrixSettingsAddRequest = req.params || null;
						resolver({ ok: true, payload: {} });
					} else if (req.method === "channels.status") {
						resolver({ ok: true, payload: { channels: [] } });
					} else {
						resolver({ ok: false, error: { message: `unexpected rpc in matrix settings test: ${req.method}` } });
					}
					delete state.pending[req.id];
				},
			});
		});

		await modal.locator('input[data-field="homeserver"]').fill("https://matrix.example.com");
		await modal.locator('select[data-field="authMode"]').selectOption("access_token");
		await expect(
			modal.getByText("Settings -> Help & About -> Advanced -> Access Token", { exact: false }),
		).toBeVisible();
		await expect(modal.getByText("Access token auth always stays user-managed", { exact: false })).toBeVisible();
		await expect(
			modal.getByText("do not transfer that device's private encryption keys into Moltis", { exact: false }),
		).toBeVisible();
		await expect(modal.getByRole("link", { name: "Matrix setup docs", exact: true })).toHaveAttribute(
			"href",
			"https://docs.moltis.org/matrix.html",
		);
		await modal.locator('input[data-field="credential"]').fill("syt_test_token");
		await modal.getByText("Advanced Config JSON", { exact: true }).click();
		await page
			.locator('textarea[data-field="advancedConfigPatch"]')
			.fill('{"reply_to_message":true,"stream_mode":"off"}');
		await page.evaluate(() => {
			const submitButton = Array.from(document.querySelectorAll(".modal-box button.provider-btn")).find(
				(button) => button.textContent?.trim() === "Connect Matrix",
			);
			if (!(submitButton instanceof HTMLButtonElement)) {
				throw new Error("visible Matrix submit button not found");
			}
			submitButton.scrollIntoView({ block: "nearest" });
			submitButton.click();
		});

		await expect.poll(() => page.evaluate(() => window.__matrixSettingsAddRequest)).not.toBeNull();

		const sentRequest = await page.evaluate(() => window.__matrixSettingsAddRequest);
		expect(sentRequest.account_id).toMatch(/^matrix-example-com-[a-z0-9]{6}$/);
		expect(sentRequest.config).toMatchObject({
			homeserver: "https://matrix.example.com",
			access_token: "syt_test_token",
			ownership_mode: "user_managed",
			auto_join: "always",
			otp_self_approval: true,
			otp_cooldown_secs: 300,
			reply_to_message: true,
			stream_mode: "off",
		});
		expect(sentRequest.config).not.toHaveProperty("user_id");
		expect(pageErrors).toEqual([]);
	});

	test("channels add matrix supports password auth and invite policy", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const addButton = page.getByRole("button", { name: "Connect Matrix", exact: true });
		await expect(addButton).toBeVisible();
		await addButton.click();

		const modal = page.locator(".modal-box");
		await expect(modal.getByRole("heading", { name: "Connect Matrix", exact: true })).toBeVisible();
		await expect(modal.locator('select[data-field="authMode"]')).toHaveValue("oidc");

		await page.evaluate(async () => {
			const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			if (!appScript) throw new Error("app.js script not found");
			const appUrl = new URL(appScript.src, window.location.origin).href;
			const marker = "js/app.js";
			const markerIdx = appUrl.indexOf(marker);
			if (markerIdx < 0) throw new Error("app.js marker not found in script URL");
			const prefix = appUrl.slice(0, markerIdx);
			const state = await import(`${prefix}js/state.js`);
			const wsOpen = typeof WebSocket !== "undefined" ? WebSocket.OPEN : 1;
			window.__matrixSettingsAddRequest = null;
			state.setConnected(true);
			state.setWs({
				readyState: wsOpen,
				send(raw) {
					const req = JSON.parse(raw || "{}");
					const resolver = state.pending[req.id];
					if (!resolver) return;
					if (req.method === "channels.add") {
						window.__matrixSettingsAddRequest = req.params || null;
						resolver({ ok: true, payload: {} });
					} else if (req.method === "channels.status") {
						resolver({ ok: true, payload: { channels: [] } });
					} else {
						resolver({ ok: false, error: { message: `unexpected rpc in matrix settings test: ${req.method}` } });
					}
					delete state.pending[req.id];
				},
			});
		});

		await modal.locator('input[data-field="homeserver"]').fill("https://matrix.example.com");
		await modal.locator('select[data-field="authMode"]').selectOption("password");
		await expect(modal.getByText("Required for encrypted Matrix chats.", { exact: false })).toBeVisible();
		await expect(modal.getByLabel("Let Moltis own this Matrix account", { exact: true })).toBeChecked();
		await modal.locator('input[data-field="userId"]').fill("@bot:example.com");
		await modal.locator('input[data-field="credential"]').fill("correct horse battery staple");
		await modal.locator('select[data-field="autoJoin"]').selectOption("allowlist");
		const matrixDmAllowlistInput = modal
			.getByText("DM Allowlist (Matrix user IDs)", { exact: true })
			.locator("xpath=following-sibling::div[1]//input");
		const matrixRoomAllowlistInput = modal
			.getByText("Room Allowlist (room IDs or aliases)", { exact: true })
			.locator("xpath=following-sibling::div[1]//input");
		await matrixDmAllowlistInput.fill("@alice:example.com");
		await matrixDmAllowlistInput.press("Enter");
		await matrixRoomAllowlistInput.fill("@ops:example.com");
		await matrixRoomAllowlistInput.press("Enter");
		await page.evaluate(() => {
			const submitButton = Array.from(document.querySelectorAll(".modal-box button.provider-btn")).find(
				(button) => button.textContent?.trim() === "Connect Matrix",
			);
			if (!(submitButton instanceof HTMLButtonElement)) {
				throw new Error("visible Matrix submit button not found");
			}
			submitButton.scrollIntoView({ block: "nearest" });
			submitButton.click();
		});

		await expect.poll(() => page.evaluate(() => window.__matrixSettingsAddRequest)).not.toBeNull();

		const sentRequest = await page.evaluate(() => window.__matrixSettingsAddRequest);
		expect(sentRequest.account_id).toBe("bot-example-com");
		expect(sentRequest.config).toMatchObject({
			homeserver: "https://matrix.example.com",
			user_id: "@bot:example.com",
			password: "correct horse battery staple",
			ownership_mode: "moltis_owned",
			auto_join: "allowlist",
			otp_self_approval: true,
			otp_cooldown_secs: 300,
			user_allowlist: ["@alice:example.com"],
			room_allowlist: ["@ops:example.com"],
		});
		expect(sentRequest.config).not.toHaveProperty("access_token");
		expect(pageErrors).toEqual([]);
	});

	test("channels page shows matrix verification state and pending verification guidance", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");

		await mockChannelsStatus(page, {
			label: "matrix status test",
			channels: [
				{
					type: "matrix",
					account_id: "moltis-testbot",
					name: "Matrix (moltis-testbot)",
					status: "connected",
					details: "@moltis-testbot:matrix.org on https://matrix.org",
					sessions: [],
					extra: {
						matrix: {
							verification_state: "unverified",
							ownership_mode: "moltis_owned",
							auth_mode: "password",
							user_id: "@moltis-testbot:matrix.org",
							device_id: "MOLTISBOT",
							device_display_name: "Moltis Matrix Bot",
							cross_signing_complete: true,
							device_verified_by_owner: false,
							recovery_state: "enabled",
							pending_verifications: [
								{
									flow_id: "flow-1",
									other_user_id: "@alice:matrix.org",
									room_id: "!room:matrix.org",
									emoji_lines: ["🐶 Dog", "🔥 Fire"],
								},
							],
						},
					},
				},
			],
		});

		await expect(page.getByText("Matrix (moltis-testbot)", { exact: true })).toBeVisible();
		await expect(page.getByText("Encryption device state: unverified", { exact: false })).toBeVisible();
		await expect(page.getByText("Managed by Moltis", { exact: true })).toBeVisible();
		await expect(page.getByText("Device not yet verified by owner", { exact: true })).toBeVisible();
		await expect(page.getByText("MOLTISBOT", { exact: true })).toBeHidden();
		const matrixDetails = page.getByText("Matrix account details", { exact: true });
		await expect(matrixDetails).toBeVisible();
		await matrixDetails.click();
		await expect(page.getByText("@moltis-testbot:matrix.org", { exact: true })).toBeVisible();
		await expect(page.getByText("MOLTISBOT", { exact: true })).toBeVisible();
		await expect(page.getByText("Verification pending", { exact: true })).toBeVisible();
		await expect(page.getByText("With @alice:matrix.org", { exact: true })).toBeVisible();
		await expect(page.getByText("verify yes", { exact: false })).toBeVisible();
		await expect(page.getByText("verify show", { exact: false })).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("channels page shows blocked Matrix ownership state for incomplete secret storage", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");

		await mockChannelsStatus(page, {
			label: "blocked matrix ownership test",
			channels: [
				{
					type: "matrix",
					account_id: "moltis-testbot",
					name: "Matrix (moltis-testbot)",
					status: "connected",
					details: "@moltis-testbot:matrix.org on https://matrix.org",
					sessions: [],
					extra: {
						matrix: {
							verification_state: "unverified",
							ownership_mode: "moltis_owned",
							auth_mode: "password",
							user_id: "@moltis-testbot:matrix.org",
							device_id: "MOLTISBOT",
							cross_signing_complete: false,
							device_verified_by_owner: false,
							recovery_state: "incomplete",
							ownership_error:
								"invalid channel input: matrix account already has incomplete secret storage that this password could not unlock; repair the account in Element or switch to user-managed mode",
							pending_verifications: [],
						},
					},
				},
			],
		});

		await expect(page.getByText("Matrix (moltis-testbot)", { exact: true })).toBeVisible();
		await expect(page.getByText("Moltis ownership blocked", { exact: true })).toBeVisible();
		await expect(
			page.getByText(
				"This account already has partial Matrix secure-backup state. Finish or repair it in Element, or switch this channel to user-managed mode.",
				{ exact: true },
			),
		).toBeVisible();
		await expect(page.getByText("Ownership setup needs attention", { exact: true })).toBeVisible();
		await expect(
			page.getByText("matrix account already has incomplete secret storage that this password could not unlock", {
				exact: false,
			}),
		).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("channels page shows Matrix ownership approval guidance for existing accounts", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");

		await mockChannelsStatus(page, {
			label: "matrix ownership approval test",
			allowRetryOwnership: true,
			channels: [
				{
					type: "matrix",
					account_id: "moltis-testbot",
					name: "Matrix (moltis-testbot)",
					status: "connected",
					details: "@moltis-testbot:matrix.org on https://matrix.org",
					sessions: [],
					extra: {
						matrix: {
							verification_state: "unverified",
							ownership_mode: "moltis_owned",
							auth_mode: "password",
							user_id: "@moltis-testbot:matrix.org",
							device_id: "GT7YDd8CWl",
							cross_signing_complete: false,
							device_verified_by_owner: false,
							recovery_state: "disabled",
							ownership_error:
								"invalid channel input: matrix account requires browser approval to reset cross-signing at https://account.matrix.org/account/?action=org.matrix.cross_signing_reset; complete that in Element or switch to user-managed mode",
							pending_verifications: [],
						},
					},
				},
			],
		});

		await expect(page.getByText("Ownership approval required", { exact: true })).toBeVisible();
		await expect(
			page.getByText(
				"This existing Matrix account can already chat, but Matrix needs one browser approval before Moltis can take over encryption ownership. Open the approval page, approve the reset, then retry ownership setup.",
				{ exact: true },
			),
		).toBeVisible();
		await expect(page.getByText("Browser approval pending", { exact: true })).toBeVisible();
		const approvalLink = page.getByRole("link", {
			name: "Open approval page for @moltis-testbot:matrix.org",
			exact: true,
		});
		await expect(approvalLink).toHaveAttribute(
			"href",
			"https://account.matrix.org/account/?action=org.matrix.cross_signing_reset",
		);
		await expect(approvalLink).toHaveClass(/provider-btn/);
		await expect(approvalLink).not.toHaveClass(/provider-btn-secondary/);
		const retryButton = page.getByRole("button", {
			name: "Click here once you reset the account",
			exact: true,
		});
		await expect(retryButton).toBeVisible();
		const approvalNote = approvalLink.locator("xpath=../following-sibling::div[1]");
		await expect(approvalNote).toContainText("Make sure the browser page is signed into @moltis-testbot:matrix.org.");
		await retryButton.click();
		await expect.poll(() => page.evaluate(() => window.__matrixOwnershipRetryRequest)).not.toBeNull();
		const retryRequest = await page.evaluate(() => window.__matrixOwnershipRetryRequest);
		expect(retryRequest).toEqual({
			type: "matrix",
			account_id: "moltis-testbot",
		});
		expect(pageErrors).toEqual([]);
	});

	test("senders page shows pending matrix sender with one visible sigil and OTP code", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");

		await mockChannelsStatus(page, {
			label: "matrix senders test",
			channels: [
				{
					type: "matrix",
					account_id: "moltis-testbot",
					name: "Matrix (moltis-testbot)",
					status: "connected",
					details: "@moltis-testbot:matrix.org on https://matrix.org",
					sessions: [],
				},
			],
			senders: [
				{
					peer_id: "@alice:matrix.org",
					username: "@alice:matrix.org",
					sender_name: "Alice",
					message_count: 1,
					last_seen: 1700000000,
					allowed: false,
					otp_pending: {
						code: "954502",
						expires_at: 1700000300,
					},
				},
			],
		});

		await expect(page.getByText("Matrix (moltis-testbot)", { exact: true })).toBeVisible({ timeout: 10_000 });
		await page.getByRole("tab", { name: /Senders/ }).click();
		await expect.poll(() => page.locator(".senders-table tbody tr").count(), { timeout: 10_000 }).toBe(1);
		await expect(page.getByText("Alice", { exact: true })).toBeVisible();
		await expect(page.getByText("@alice:matrix.org", { exact: true })).toBeVisible();
		await expect(page.getByText("@@alice:matrix.org", { exact: true })).toHaveCount(0);
		await expect(page.getByText("954502", { exact: true })).toBeVisible();
		await expect(page.getByText("Approve", { exact: true })).toBeVisible();
		expect(pageErrors).toEqual([]);
	});
});
