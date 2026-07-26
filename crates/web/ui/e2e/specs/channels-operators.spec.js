const { expect, test } = require("../base-test");
const { navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

// Inject a WebSocket stub serving a single configured channel and capturing
// the channels.update payload the edit modal sends on save.
async function installChannelMock(page, channel) {
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
		window.__channelUpdateRequest = null;
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
					window.__channelUpdateRequest = req.params || null;
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

function discordChannel(config) {
	return {
		type: "discord",
		account_id: "test-bot",
		name: "Test Discord",
		status: "connected",
		config,
	};
}

async function openEditModal(page, channel) {
	await installChannelMock(page, channel);
	await expect(page.getByText("Test Discord", { exact: true })).toBeVisible({ timeout: 10_000 });
	await page.locator('button[title="Edit test-bot"]').click();
	const modal = page.locator(".modal-box");
	await expect(modal.getByText("Operators", { exact: true })).toBeVisible();
	return modal;
}

// The Operators list decides who may use /sh and host-reaching tools, so the
// edit modal must load it, save it, and state which rule is currently in force.
test.describe("Channel operators", () => {
	test("operators round-trip through the edit modal", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const modal = await openEditModal(
			page,
			discordChannel({ allowlist: ["owner-id"], operators: ["owner-id"] }),
		);

		// Existing operator renders as a tag.
		await expect(modal.getByText("owner-id", { exact: true }).first()).toBeVisible();
		await expect(modal.getByTestId("operators-hint")).toContainText("Only these senders can use /sh");

		// Add a second operator via the tag input.
		const operatorsInput = modal
			.locator("label", { hasText: "Operators" })
			.locator("xpath=following-sibling::div[1]")
			.locator("input");
		await operatorsInput.fill("trusted-admin");
		await operatorsInput.press("Enter");

		await modal.getByRole("button", { name: "Save Changes", exact: true }).click();

		await expect.poll(() => page.evaluate(() => window.__channelUpdateRequest)).toMatchObject({
			config: { operators: ["owner-id", "trusted-admin"] },
		});

		expect(pageErrors).toEqual([]);
	});

	test("hint explains the allowlist fallback when no operators are set", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const modal = await openEditModal(page, discordChannel({ allowlist: ["owner-id"], operators: [] }));

		await expect(modal.getByTestId("operators-hint")).toContainText("the DM allowlist above is used instead");

		expect(pageErrors).toEqual([]);
	});

	test("hint warns that shell access is off when nothing is configured", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const modal = await openEditModal(page, discordChannel({ allowlist: [], operators: [] }));

		await expect(modal.getByTestId("operators-hint")).toContainText(
			"shell access and host tools are disabled for every sender",
		);

		expect(pageErrors).toEqual([]);
	});
});
