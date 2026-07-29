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
		const handlers = {
			"channels.status": () => ({ channels: [mockChannel] }),
			"channels.senders.list": () => ({ senders: mockChannel.__senders || [] }),
			"agents.list": () => ({ agents: [] }),
			"channels.senders.approve": (params) => {
				window.__senderApproveRequest = params || null;
				return {};
			},
			"channels.update": (params) => {
				window.__channelUpdateRequest = params || null;
				return {};
			},
		};
		state.setWs({
			readyState: wsOpen,
			send(raw) {
				const req = JSON.parse(raw || "{}");
				const resolver = state.pending[req.id];
				if (!resolver) return;
				const handler = handlers[req.method];
				resolver({ ok: true, payload: handler ? handler(req.params) : {} });
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

// The tag list rendered by <AllowlistInput> for the Operators field. Scoping to
// it keeps assertions off the Allowlist field above, which holds the same IDs.
function operatorsField(modal) {
	return modal.locator("label", { hasText: "Operators" }).locator("xpath=following-sibling::div[1]");
}

// The Operators list decides who may use /sh and host-reaching tools, so the
// edit modal must load it, save it, and state which rule is currently in force.
test.describe("Channel operators", () => {
	test("operators round-trip through the edit modal", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const modal = await openEditModal(page, discordChannel({ allowlist: ["owner-id"], operators: ["owner-id"] }));

		// Existing operator renders as a tag. Each tag is a <span> holding the
		// value plus a "×" remove button, so match on the tag rather than on
		// text equal to the value alone.
		const operators = operatorsField(modal);
		await expect(operators.getByText("owner-id")).toBeVisible();
		await expect(modal.getByTestId("operators-hint")).toContainText("Only these exact sender IDs");

		// Add a second operator via the tag input.
		const operatorsInput = operators.locator("input");
		await operatorsInput.fill("trusted-admin");
		await operatorsInput.press("Enter");

		await modal.getByRole("button", { name: "Save Changes", exact: true }).click();

		await expect
			.poll(() => page.evaluate(() => window.__channelUpdateRequest))
			.toMatchObject({
				config: { operators: ["owner-id", "trusted-admin"] },
			});

		expect(pageErrors).toEqual([]);
	});

	test("hint explains that an empty operator list fails closed", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const modal = await openEditModal(page, discordChannel({ allowlist: ["owner-id"], operators: [] }));

		await expect(modal.getByTestId("operators-hint")).toContainText(
			"privileged commands and private host access are disabled for every sender",
		);

		expect(pageErrors).toEqual([]);
	});

	test("hint warns that shell access is off when nothing is configured", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const modal = await openEditModal(page, discordChannel({ allowlist: [], operators: [] }));

		await expect(modal.getByTestId("operators-hint")).toContainText("disabled for every sender");

		expect(pageErrors).toEqual([]);
	});
});

// Approving a sender for access and granting them the host are separate
// decisions, so "Approve" must always ask which one is meant.
test.describe("Sender approval role", () => {
	async function openApprovalDialog(page, config) {
		const channel = discordChannel(config);
		channel.__senders = [
			{ peer_id: "400347514466992128", sender_name: "Fabien", username: "fabien", message_count: 2, allowed: false },
		];
		await installChannelMock(page, channel);
		// The tab bar renders role="tab", and the badge puts the count in the
		// accessible name, so match on the label rather than the exact string.
		await page.getByRole("tab", { name: /Senders/ }).click();
		await page.getByRole("button", { name: "Approve", exact: true }).click();
		return page.locator(".modal-box");
	}

	test("operator is preselected while the channel has no operators", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const modal = await openApprovalDialog(page, { allowlist: [], operators: [] });

		await expect(modal.getByTestId("approve-role-operator")).toHaveAttribute("data-selected", "true");
		await expect(modal.getByTestId("approve-role-guest")).toHaveAttribute("data-selected", "false");
		await expect(modal.getByTestId("approve-first-operator-hint")).toBeVisible();

		// The operator grant must carry peer_id: operators match exact platform
		// IDs, so approving on the username alone would never take effect.
		await modal.getByRole("button", { name: "Approve as operator", exact: true }).click();
		await expect
			.poll(() => page.evaluate(() => window.__senderApproveRequest))
			.toMatchObject({
				identifier: "fabien",
				peer_id: "400347514466992128",
				role: "operator",
			});

		expect(pageErrors).toEqual([]);
	});

	test("guest is the default once an operator exists", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const modal = await openApprovalDialog(page, { allowlist: [], operators: ["someone-else"] });

		await expect(modal.getByTestId("approve-role-guest")).toHaveAttribute("data-selected", "true");
		await expect(modal.getByTestId("approve-role-operator")).toHaveAttribute("data-selected", "false");
		await expect(modal.getByTestId("approve-first-operator-hint")).toHaveCount(0);

		await modal.getByRole("button", { name: "Approve", exact: true }).click();
		await expect.poll(() => page.evaluate(() => window.__senderApproveRequest)).toMatchObject({ role: "guest" });

		expect(pageErrors).toEqual([]);
	});
});
