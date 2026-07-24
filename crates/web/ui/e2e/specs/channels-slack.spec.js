const { expect, test } = require("../base-test");
const { navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

// Inject a WebSocket stub that serves a single configured Slack channel and
// captures the channels.update payload the edit modal sends on save.
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
		state.setConnected(true);
		state.setWs({
			readyState: wsOpen,
			send(raw) {
				const req = JSON.parse(raw || "{}");
				const resolver = state.pending[req.id];
				if (!resolver) return;
				if (req.method === "channels.status") {
					resolver({ ok: true, payload: { channels: [mockChannel] } });
				} else if (req.method === "channels.senders.list") {
					resolver({ ok: true, payload: { senders: [] } });
				} else if (req.method === "agents.list") {
					resolver({ ok: true, payload: { agents: [] } });
				} else if (req.method === "channels.update") {
					window.__slackUpdateRequest = req.params || null;
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

		// Reflects current config: ack on, triggers off, rich blocks off.
		await expect(ackCheckbox).toBeChecked();
		await expect(triggerCheckbox).not.toBeChecked();
		await expect(richBlocksCheckbox).not.toBeChecked();

		// Flip all three.
		await ackCheckbox.uncheck();
		await triggerCheckbox.check();
		await richBlocksCheckbox.check();

		await modal.getByRole("button", { name: "Save Changes", exact: true }).click();

		await expect
			.poll(() => page.evaluate(() => window.__slackUpdateRequest))
			.toMatchObject({ config: { ack_reactions: false, reaction_triggers: true, rich_blocks: true } });

		expect(pageErrors).toEqual([]);
	});
});
