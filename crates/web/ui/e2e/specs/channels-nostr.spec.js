const { expect, test } = require("../base-test");
const { navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

test.describe("Nostr channel", () => {
	test("connect button visible when nostr is offered", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const addButton = page.getByRole("button", { name: "Connect Nostr", exact: true });
		await expect(addButton).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("connect button renders a nostr icon mask", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const addButton = page.getByRole("button", { name: "Connect Nostr", exact: true });
		await expect(addButton).toBeVisible();

		const icon = addButton.locator(".icon.icon-nostr");
		await expect(icon).toBeVisible();
		await expect
			.poll(() =>
				icon.evaluate((node) => {
					const style = window.getComputedStyle(node);
					return style.maskImage || style.webkitMaskImage || "";
				}),
			)
			.not.toBe("none");

		expect(pageErrors).toEqual([]);
	});

	test("add modal has secret key, relays, and DM policy fields", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const addButton = page.getByRole("button", { name: "Connect Nostr", exact: true });
		await addButton.click();

		await expect(page.getByRole("heading", { name: "Connect Nostr", exact: true })).toBeVisible();

		// Secret key field is a password input
		const secretKeyInput = page.getByPlaceholder("nsec1... or 64-char hex");
		await expect(secretKeyInput).toBeVisible();
		await expect(secretKeyInput).toHaveAttribute("type", "password");
		await expect(secretKeyInput).toHaveAttribute("autocomplete", "new-password");
		await expect(secretKeyInput).toHaveAttribute("name", "nostr_secret_key");

		// Account ID field
		const accountIdInput = page.locator('input[data-field="accountId"]');
		await expect(accountIdInput).toBeVisible();

		// Relays field with default value
		const relaysInput = page.locator('input[data-field="relays"]');
		await expect(relaysInput).toBeVisible();
		const relaysValue = await relaysInput.inputValue();
		expect(relaysValue).toContain("relay.damus.io");
		expect(relaysValue).toContain("nos.lol");

		// DM policy select
		const dmPolicySelect = page.locator('select[data-field="dmPolicy"]');
		await expect(dmPolicySelect).toBeVisible();
		await expect(dmPolicySelect).toHaveValue("allowlist");

		// Setup instructions
		await expect(page.getByText("How to set up Nostr DMs")).toBeVisible();
		await expect(page.getByText("Generate or use an existing Nostr secret key")).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("add modal validates required fields", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		await page.getByRole("button", { name: "Connect Nostr", exact: true }).click();
		await expect(page.getByRole("heading", { name: "Connect Nostr", exact: true })).toBeVisible();

		// Click connect without filling fields
		await page.getByRole("button", { name: "Connect Nostr", exact: false }).last().click();
		await expect(page.getByText("Account ID is required.")).toBeVisible();

		// Fill account ID but not secret key
		await page.locator('input[data-field="accountId"]').fill("test-nostr");
		await page.getByRole("button", { name: "Connect Nostr", exact: false }).last().click();
		await expect(page.getByText("Secret key is required.")).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("nostr channel displayed in channel list", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		// Mock channels.status to return a nostr channel
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
			state.setConnected(true);
			state.setWs({
				readyState: wsOpen,
				send(raw) {
					const req = JSON.parse(raw || "{}");
					const resolver = state.pending[req.id];
					if (!resolver) return;
					if (req.method === "channels.status") {
						resolver({
							ok: true,
							payload: {
								channels: [
									{
										type: "nostr",
										account_id: "my-nostr-bot",
										name: "my-nostr-bot",
										status: "connected",
										details: "2/3 relays connected",
										sessions: [],
										config: {
											dm_policy: "allowlist",
											allowed_pubkeys: ["npub1test123"],
											relays: ["wss://relay.damus.io", "wss://nos.lol", "wss://relay.nostr.band"],
										},
									},
								],
							},
						});
					} else if (req.method === "channels.senders.list") {
						resolver({ ok: true, payload: { senders: [] } });
					}
				},
			});
			const channelsPage = await import(`${prefix}js/page-channels.js`);
			await channelsPage.prefetchChannels();
		});

		// Wait for the channel to appear
		await expect(page.getByText("my-nostr-bot")).toBeVisible({ timeout: 5000 });
		await expect(page.getByText("2/3 relays connected")).toBeVisible();

		expect(pageErrors).toEqual([]);
	});
});
