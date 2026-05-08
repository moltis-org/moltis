const { expect, test } = require("../base-test");
const { navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

async function mockPhoneProviders(page, providers) {
	await expect.poll(() => new URL(page.url()).pathname).toBe("/settings/phone");
	await expect(page.getByRole("heading", { name: "Phone", exact: true })).toBeVisible();
	await page.waitForFunction(() => !!document.querySelector('script[type="module"][src*="js/app.js"]'));
	await page.evaluate(async (mockProviders) => {
		const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
		if (!appScript) throw new Error("app.js script not found");
		const appUrl = new URL(appScript.src, window.location.origin).href;
		const marker = "js/app.js";
		const markerIdx = appUrl.indexOf(marker);
		if (markerIdx < 0) throw new Error("app.js marker not found in script URL");
		const prefix = appUrl.slice(0, markerIdx);
		const state = await import(`${prefix}js/state.js`);
		const wsOpen = typeof WebSocket !== "undefined" ? WebSocket.OPEN : 1;
		window.__phoneSettingsRequests = [];
		state.setConnected(false);
		state.setWs({
			readyState: wsOpen,
			send(raw) {
				const req = JSON.parse(raw || "{}");
				const resolver = state.pending[req.id];
				if (!resolver) return;
				window.__phoneSettingsRequests.push({
					method: req.method,
					params: req.params || {},
				});
				if (req.method === "phone.providers.all") {
					resolver({ ok: true, payload: { providers: mockProviders } });
				} else if (
					req.method === "phone.config.save_key" ||
					req.method === "phone.config.save_settings" ||
					req.method === "phone.provider.toggle" ||
					req.method === "phone.config.remove_key"
				) {
					resolver({ ok: true, payload: { ok: true } });
				} else {
					resolver({
						ok: false,
						error: { message: `unexpected rpc in phone settings test: ${req.method}` },
					});
				}
				delete state.pending[req.id];
			},
		});
		state.setConnected(true);
	}, providers);
}

test.describe("Settings > Phone", () => {
	const providers = [
		{
			id: "twilio",
			name: "Twilio",
			type: "telephony",
			category: "Cloud",
			description: "Make and receive phone calls via the Twilio API.",
			available: false,
			enabled: false,
			keyPlaceholder: "AC...",
			keyUrl: "https://www.twilio.com/console",
			keyUrlLabel: "Twilio Console",
			hint: "Requires Account SID, Auth Token, and a phone number",
			settings: { from_number: "", webhook_url: "" },
		},
		{
			id: "telnyx",
			name: "Telnyx",
			type: "telephony",
			category: "Cloud",
			description: "Developer-friendly telephony.",
			available: false,
			enabled: false,
			keyPlaceholder: "KEY_...",
			keyUrl: "https://portal.telnyx.com",
			keyUrlLabel: "Telnyx Portal",
			hint: "Requires API Key, Connection ID, and a phone number",
			settings: { from_number: "", webhook_url: "", connection_id: "" },
		},
	];

	test("phone providers live under the separate Phone settings page", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/phone");
		await waitForWsConnected(page);
		await mockPhoneProviders(page, providers);

		await expect(page.getByText("Twilio", { exact: true })).toBeVisible();
		await expect(page.getByText("Telnyx", { exact: true })).toBeVisible();
		await expect(page.getByRole("button", { name: "Connect Phone Calls", exact: true })).not.toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("channels page does not expose phone account setup", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		await expect(page.getByRole("heading", { name: "Channels", exact: true })).toBeVisible();
		await expect(page.getByRole("button", { name: "Connect Phone Calls", exact: true })).not.toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("telnyx setup uses API Key and Connection ID labels", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/phone");
		await waitForWsConnected(page);
		await mockPhoneProviders(page, providers);

		const telnyxCard = page
			.locator("div.rounded-lg")
			.filter({ has: page.getByText("Telnyx", { exact: true }) })
			.first();
		await telnyxCard.getByRole("button", { name: "Set up credentials", exact: true }).click();
		const modal = page.locator(".modal-box").filter({ has: page.getByText("Configure Telnyx", { exact: true }) });
		await expect(modal).toBeVisible();
		await expect(modal.getByText("API Key", { exact: true })).toBeVisible();
		await expect(modal.getByText("Connection ID", { exact: true })).toBeVisible();
		await expect(modal.locator('input[placeholder="KEY_..."]')).toHaveAttribute("type", "password");
		await expect(modal.locator('input[placeholder="Call Control connection ID"]')).toHaveAttribute("type", "text");

		await modal.locator('input[placeholder="KEY_..."]').fill("KEY_test");
		await modal.locator('input[placeholder="Call Control connection ID"]').fill("conn_test");
		await modal.locator('input[placeholder="+15551234567"]').fill("+15551234567");
		await modal.locator('input[placeholder="https://your-domain.com"]').fill("https://phone.example.com");
		await modal.getByRole("button", { name: "Save", exact: true }).click();

		await expect
			.poll(() =>
				page.evaluate(
					() => window.__phoneSettingsRequests.find((req) => req.method === "phone.config.save_key") || null,
				),
			)
			.not.toBeNull();
		const saveRequest = await page.evaluate(() =>
			window.__phoneSettingsRequests.find((req) => req.method === "phone.config.save_key"),
		);
		expect(saveRequest.params).toMatchObject({
			provider: "telnyx",
			account_sid: "KEY_test",
			auth_token: "conn_test",
			from_number: "+15551234567",
			webhook_url: "https://phone.example.com",
		});
		expect(pageErrors).toEqual([]);
	});
});
