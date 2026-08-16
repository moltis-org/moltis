const { expect, test } = require("../base-test");
const { navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

// Inject a WebSocket stub that serves an optional Slack channel and captures
// the channels.add/channels.update payloads sent by the settings modals.
async function installSlackChannelMock(page, channel) {
	await page.evaluate(async (mockChannel) => {
		const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
		if (!appScript) throw new Error("app.js script not found");
		const appUrl = new URL(appScript.src, window.location.origin).href;
		const marker = "js/app.js";
		const markerIdx = appUrl.indexOf(marker);
		if (markerIdx < 0) throw new Error("app.js marker not found in script URL");
		const prefix = appUrl.slice(0, markerIdx);
		const state = await import(`${prefix}js/state.js`);
		const channelsPage = await import(`${prefix}js/page-channels.js`);
		const wsOpen = typeof WebSocket === "undefined" ? 1 : WebSocket.OPEN;
		window.__slackUpdateRequest = null;
		window.__slackAddRequest = null;
		state.setConnected(true);
		state.setWs({
			readyState: wsOpen,
			send(raw) {
				const req = JSON.parse(raw || "{}");
				const resolver = state.pending[req.id];
				if (!resolver) return;
				if (req.method === "channels.status") {
					resolver({ ok: true, payload: { channels: mockChannel ? [mockChannel] : [] } });
				} else if (req.method === "channels.senders.list") {
					resolver({ ok: true, payload: { senders: [] } });
				} else if (req.method === "agents.list") {
					resolver({ ok: true, payload: { agents: [] } });
				} else if (req.method === "channels.update") {
					window.__slackUpdateRequest = req.params || null;
					resolver({ ok: true, payload: {} });
				} else if (req.method === "channels.add") {
					window.__slackAddRequest = req.params || null;
					resolver({ ok: true, payload: {} });
				} else {
					resolver({ ok: true, payload: {} });
				}
				delete state.pending[req.id];
			},
		});
		if (typeof state.refreshChannelsPage === "function") {
			state.refreshChannelsPage();
		} else {
			await channelsPage.prefetchChannels();
		}
		await new Promise((resolve) => setTimeout(resolve, 100));
	}, channel);
}

test.describe("Slack channel settings", () => {
	test("setup guide lists required scopes, events, and Events API routes", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);
		await installSlackChannelMock(page, null);

		await page.getByRole("button", { name: "Connect Slack", exact: true }).click();
		const modal = page.locator(".modal-box");
		const guide = page.getByTestId("slack-setup-guide");
		await expect(guide).toBeVisible();

		const scopes = [
			"app_mentions:read",
			"chat:write",
			"files:write",
			"im:history",
			"reactions:write",
			"reactions:read",
			"channels:history",
			"groups:history",
			"mpim:history",
			"connections:write",
		];
		for (const scope of scopes) {
			await expect(guide.getByText(scope, { exact: true })).toBeVisible();
		}

		const events = [
			"app_mention",
			"message.im",
			"reaction_added",
			"message.channels",
			"message.groups",
			"message.mpim",
		];
		for (const event of events) {
			await expect(guide.getByText(event, { exact: true })).toBeVisible();
		}

		const routes = [
			"https://your-host/api/channels/slack/<id>/events",
			"https://your-host/api/channels/slack/<id>/interactions",
			"https://your-host/api/channels/slack/<id>/commands",
		];
		for (const route of routes) {
			await expect(guide.getByText(route, { exact: true })).toBeVisible();
		}

		await expect(guide).toContainText("mention_mode = always");
		await expect(guide).toContainText("Each scope permits access; its paired event delivers messages");
		const streaming = page.getByRole("combobox", { name: "Response streaming", exact: true });
		await expect(streaming).toHaveValue("edit_in_place");
		await expect(streaming.locator('option[value="native"]')).toHaveText("Slack live text and tool cards");

		await modal.locator('[data-field="accountId"]').fill("test-add");
		await modal.locator('[data-field="botToken"]').fill("xoxb-test");
		await modal.locator('[data-field="appToken"]').fill("xapp-test");
		await modal.getByText("Advanced Config JSON", { exact: true }).click();
		await modal
			.locator('[data-field="advancedConfigPatch"]')
			.fill('{"stream_mode":"native","thread_replies":false,"rich_blocks":true}');
		await modal.getByRole("button", { name: "Connect Slack", exact: true }).click();
		await expect
			.poll(() => page.evaluate(() => window.__slackAddRequest))
			.toMatchObject({
				account_id: "test-add",
				config: { stream_mode: "native", thread_replies: true, rich_blocks: false },
			});
		expect(pageErrors).toEqual([]);
	});

	test("ack_reactions and reaction_triggers toggles round-trip through the edit modal", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		await installSlackChannelMock(page, {
			type: "slack",
			account_id: "test-bot",
			name: "Test Slack",
			status: "connected",
			config: {
				api_base_url: "https://slack.com/api",
				stream_mode: "edit_in_place",
				thread_replies: false,
				ack_reactions: true,
				reaction_triggers: false,
			},
		});

		// Card renders from the mocked channels.status response.
		await expect(page.getByText("Test Slack", { exact: true })).toBeVisible({ timeout: 10_000 });

		// Open the edit modal for the mocked Slack channel.
		await page.locator('button[title="Edit test-bot"]').click();
		const modal = page.locator(".modal-box");
		await expect(modal.getByText("Acknowledge with reactions", { exact: true })).toBeVisible();

		const ackCheckbox = modal
			.locator("label", { hasText: "Acknowledge with reactions" })
			.locator('input[type="checkbox"]');
		const triggerCheckbox = modal.locator("label", { hasText: "Reaction triggers" }).locator('input[type="checkbox"]');
		const richBlocksCheckbox = modal
			.locator("label", { hasText: "Rich Block Kit rendering" })
			.locator('input[type="checkbox"]');
		const streaming = modal.getByRole("combobox", { name: "Response streaming", exact: true });

		// Reflects current config: ack on, triggers off, rich blocks off.
		await expect(ackCheckbox).toBeChecked();
		await expect(triggerCheckbox).not.toBeChecked();
		await expect(richBlocksCheckbox).not.toBeChecked();
		await expect(streaming).toHaveValue("edit_in_place");

		// Exercise the visible native-mode exclusivity, then reproduce an advanced
		// patch introducing native mode with conflicting prerequisites.
		await ackCheckbox.uncheck();
		await triggerCheckbox.check();
		await richBlocksCheckbox.check();
		await streaming.selectOption("native");
		await streaming.selectOption("edit_in_place");
		await modal.getByText("Advanced Config JSON", { exact: true }).click();
		await modal
			.locator('[data-field="advancedConfigPatch"]')
			.fill('{"stream_mode":"native","thread_replies":false,"rich_blocks":true}');

		await modal.getByRole("button", { name: "Save Changes", exact: true }).click();

		await expect
			.poll(() => page.evaluate(() => window.__slackUpdateRequest))
			.toMatchObject({
				config: {
					ack_reactions: false,
					reaction_triggers: true,
					rich_blocks: false,
					stream_mode: "native",
					thread_replies: true,
				},
			});

		expect(pageErrors).toEqual([]);
	});
});
