const { expect, test } = require("../base-test");
const { navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

async function installWhatsAppChannelMock(page, channel) {
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
		window.__whatsAppUpdateRequest = null;
		state.setConnected(true);
		const responseFor = (req) => {
			const responses = {
				"channels.status": () => ({ ok: true, payload: { channels: [mockChannel] } }),
				"channels.senders.list": () => ({ ok: true, payload: { senders: [] } }),
				"agents.list": () => ({ ok: true, payload: { agents: [] } }),
				"channels.update": () => {
					window.__whatsAppUpdateRequest = req.params || null;
					return { ok: true, payload: {} };
				},
			};
			return responses[req.method]?.() || { ok: true, payload: {} };
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
		if (typeof state.refreshChannelsPage === "function") {
			state.refreshChannelsPage();
		} else {
			await channelsPage.prefetchChannels();
		}
	}, channel);
}

test.describe("WhatsApp channel settings", () => {
	test("inbound document downloads are opt-in and round-trip through edit", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);
		await installWhatsAppChannelMock(page, {
			type: "whatsapp",
			account_id: "test-account",
			name: "Test WhatsApp",
			status: "connected",
			config: {
				dm_policy: "allowlist",
				download_inbound_documents: false,
			},
		});

		await expect(page.getByText("Test WhatsApp", { exact: true })).toBeVisible({ timeout: 10_000 });
		await page.getByRole("button", { name: "Edit", exact: true }).click();

		const modal = page.locator(".modal-box");
		const checkbox = modal.getByRole("checkbox", { name: "Download inbound documents", exact: true });
		await expect(checkbox).not.toBeChecked();
		await expect(modal.getByText("Disabled by default.", { exact: false })).toBeVisible();

		await checkbox.check();
		await modal.getByRole("button", { name: "Save Changes", exact: true }).click();

		await expect
			.poll(() => page.evaluate(() => window.__whatsAppUpdateRequest))
			.toMatchObject({
				account_id: "test-account",
				config: { download_inbound_documents: true },
			});
		expect(pageErrors).toEqual([]);
	});
});
