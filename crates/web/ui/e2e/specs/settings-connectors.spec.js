const { expect, test } = require("../base-test");
const { navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

async function installConnectorsRpcMock(page) {
	await expect(page.getByRole("heading", { name: "Connectors", exact: true })).toBeVisible();
	await page.waitForFunction(() => Boolean(document.querySelector('script[type="module"][src*="js/app.js"]')));
	await page.evaluate(async () => {
		const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
		if (!appScript) throw new Error("app.js script not found");
		const appUrl = new URL(appScript.src, window.location.origin).href;
		const markerIndex = appUrl.indexOf("js/app.js");
		if (markerIndex < 0) throw new Error("app.js marker not found");
		const stateModule = await import(`${appUrl.slice(0, markerIndex)}js/state.js`);
		const wsOpen = typeof WebSocket === "undefined" ? 1 : WebSocket.OPEN;
		const now = "2026-08-06T10:00:00Z";
		window.__connectorRpcState = {
			accounts: [],
			datasets: [],
			runs: [],
			items: [],
			requests: [],
			calendars: [
				{
					href: "/dav/calendars/alice/work/",
					displayName: "Work",
					color: "#3366ff",
					description: "Team meetings",
					collectionEtag: "etag-work",
					supportsSync: true,
				},
				{
					href: "/dav/calendars/alice/personal/",
					displayName: "Personal",
					supportsSync: false,
				},
			],
		};

		// Keep every mutable contract operation in one dispatcher so request recording and state changes stay atomic.
		// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: the switch mirrors the finalized RPC surface.
		function responseFor(request) {
			const mock = window.__connectorRpcState;
			const params = request.params || {};
			mock.requests.push({ method: request.method, params: structuredClone(params) });
			switch (request.method) {
				case "connectors.available":
					return { ok: true, payload: { connectors: [{ kind: "caldav", displayName: "CalDAV" }] } };
				case "connectors.accounts.list":
					return { ok: true, payload: { accounts: structuredClone(mock.accounts) } };
				case "connectors.accounts.add": {
					const account = {
						id: `account-${mock.accounts.length + 1}`,
						kind: "caldav",
						name: params.name,
						serverUrl: params.serverUrl,
						username: params.username,
						timeoutSeconds: params.timeoutSeconds,
						allowInsecureHttp: params.allowInsecureHttp,
						allowPrivateNetwork: params.allowPrivateNetwork,
						hasPassword: Boolean(params.password),
						managed: false,
						enabled: params.enabled,
						createdAt: now,
						updatedAt: now,
					};
					mock.accounts.push(account);
					return { ok: true, payload: structuredClone(account) };
				}
				case "connectors.accounts.update": {
					const account = mock.accounts.find((candidate) => candidate.id === params.id);
					if (!account) return { ok: false, error: { message: "account not found" } };
					Object.assign(account, {
						name: params.name,
						serverUrl: params.serverUrl,
						username: params.username,
						timeoutSeconds: params.timeoutSeconds,
						allowInsecureHttp: params.allowInsecureHttp,
						allowPrivateNetwork: params.allowPrivateNetwork,
						enabled: params.enabled,
						updatedAt: now,
					});
					if (params.password) account.hasPassword = true;
					return { ok: true, payload: structuredClone(account) };
				}
				case "connectors.accounts.remove":
					mock.accounts = mock.accounts.filter((account) => account.id !== params.id);
					mock.datasets = mock.datasets.filter((dataset) => dataset.accountId !== params.id);
					return { ok: true, payload: { removed: true } };
				case "connectors.accounts.test":
					return { ok: true, payload: { calendars: structuredClone(mock.calendars) } };
				case "connectors.datasets.list":
					return { ok: true, payload: { datasets: structuredClone(mock.datasets) } };
				case "connectors.datasets.compile": {
					const scheduleMinutes = params.overrides?.scheduleMinutes ?? 60;
					return {
						ok: true,
						payload: {
							draft: {
								name: "Calendar archive",
								config: {
									schemaVersion: 1,
									selection: { mode: "all" },
									filters: {
										startDate: "2026-08-01",
										endDate: "2026-12-31",
										acceptedByAccount: true,
									},
								},
								scheduleMinutes,
								projections: { jsonl: true, markdown: true },
								enabled: true,
							},
							summary: "Accepted work events will be kept locally and exported in both formats.",
							warnings: ["New calendars added later will also be included."],
						},
					};
				}
				case "connectors.datasets.add": {
					const dataset = {
						id: `dataset-${mock.datasets.length + 1}`,
						accountId: params.accountId,
						name: params.name,
						instruction: params.instruction,
						config: structuredClone(params.config),
						scheduleMinutes: params.scheduleMinutes,
						projections: structuredClone(params.projections),
						enabled: params.enabled,
						itemCount: 0,
						needsSync: true,
						createdAt: now,
						updatedAt: now,
					};
					mock.datasets.push(dataset);
					return { ok: true, payload: structuredClone(dataset) };
				}
				case "connectors.datasets.update": {
					const dataset = mock.datasets.find((candidate) => candidate.id === params.id);
					if (!dataset) return { ok: false, error: { message: "dataset not found" } };
					Object.assign(dataset, {
						name: params.name,
						instruction: params.instruction,
						config: structuredClone(params.config),
						scheduleMinutes: params.scheduleMinutes,
						projections: structuredClone(params.projections),
						enabled: params.enabled,
						needsSync: true,
						updatedAt: now,
					});
					return { ok: true, payload: structuredClone(dataset) };
				}
				case "connectors.datasets.remove":
					mock.datasets = mock.datasets.filter((dataset) => dataset.id !== params.id);
					mock.runs = mock.runs.filter((run) => run.datasetId !== params.id);
					mock.items = mock.items.filter((item) => item.datasetId !== params.id);
					return { ok: true, payload: { removed: true } };
				case "connectors.datasets.sync": {
					const dataset = mock.datasets.find((candidate) => candidate.id === params.id);
					if (!dataset) return { ok: false, error: { message: "dataset not found" } };
					const run = {
						id: `run-${mock.runs.length + 1}`,
						datasetId: dataset.id,
						status: "succeeded",
						startedAt: now,
						finishedAt: "2026-08-06T10:00:03Z",
						upserted: 1,
						deleted: 0,
						active: 1,
					};
					mock.runs.unshift(run);
					dataset.itemCount = 1;
					dataset.lastSyncAt = run.finishedAt;
					dataset.nextSyncAt = "2026-08-06T11:00:03Z";
					dataset.needsSync = false;
					mock.items = [
						{
							id: "item-1",
							datasetId: dataset.id,
							remoteId: "event-1",
							kind: "calendar_event",
							remoteVersion: "v1",
							occurredAt: "2026-08-07T09:00:00Z",
							updatedAt: now,
							bodyJson: { summary: "Planning review", attendees: ["Alice", "Bob"] },
							contentHash: "hash-1",
							createdAt: now,
							storedAt: now,
						},
					];
					return { ok: true, payload: structuredClone(run) };
				}
				case "connectors.runs.list":
					return {
						ok: true,
						payload: {
							runs: structuredClone(
								mock.runs.filter((run) => run.datasetId === params.datasetId).slice(0, params.limit),
							),
						},
					};
				case "connectors.items.query":
					return {
						ok: true,
						payload: {
							items: structuredClone(
								mock.items
									.filter((item) => item.datasetId === params.datasetId)
									.slice(params.offset, params.offset + params.limit),
							),
						},
					};
				default:
					return { ok: true, payload: {} };
			}
		}

		stateModule.setWs({
			readyState: wsOpen,
			send(raw) {
				const request = JSON.parse(raw || "{}");
				const resolver = stateModule.pending[request.id];
				if (!resolver) return;
				resolver(responseFor(request));
				delete stateModule.pending[request.id];
			},
		});
		stateModule.setConnected(true);
	});
	await page.getByRole("button", { name: "Refresh", exact: true }).click();
	await expect(page.getByRole("button", { name: "Add CalDAV connection", exact: true })).toBeEnabled();
}

function cardForHeading(page, name) {
	return page
		.getByRole("heading", { name, exact: true })
		.locator("xpath=ancestor::div[contains(@class, 'rounded-lg')][1]");
}

test.describe("Settings > Connectors", () => {
	test("hides connector settings when the server feature is disabled", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.addInitScript(() => {
			let gonValue = { connectors_enabled: false };
			Object.defineProperty(window, "__MOLTIS__", {
				configurable: true,
				get() {
					return gonValue;
				},
				set(value) {
					gonValue = { ...(value || {}), connectors_enabled: false };
				},
			});
		});

		await navigateAndWait(page, "/settings/connectors");
		await expect(page).toHaveURL(/\/settings\/profile$/);
		await expect(page.locator('.settings-nav-item[data-section="connectors"]')).toHaveCount(0);

		await page.keyboard.press("Control+k");
		await page.locator(".cmd-palette-input").fill("caldav");
		await expect(page.locator(".cmd-palette-item").filter({ hasText: "Connectors" })).toHaveCount(0);
		expect(pageErrors).toEqual([]);
	});

	test("manages CalDAV connections, datasets, previews, and activity", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		const password = "correct-horse-private-secret";
		await navigateAndWait(page, "/settings/connectors");
		await expect(page).toHaveURL(/\/settings\/connectors$/);
		await waitForWsConnected(page);
		await installConnectorsRpcMock(page);

		await page.getByRole("button", { name: "Add CalDAV connection", exact: true }).click();
		let modal = page.locator(".modal-box").filter({
			has: page.getByRole("heading", { name: "Add CalDAV connection", exact: true }),
		});
		await modal.getByLabel("Connection name", { exact: true }).fill("Work CalDAV");
		await modal.getByLabel("Server URL", { exact: true }).fill("https://cal.example.test/dav");
		await modal.getByLabel("Username", { exact: true }).fill("alice@example.test");
		await modal.getByLabel("Password", { exact: true }).fill(password);
		await modal.getByText("Advanced network settings", { exact: true }).click();
		await modal.getByLabel("Timeout (seconds)", { exact: true }).fill("301");
		await modal.getByRole("button", { name: "Add connection", exact: true }).click();
		await expect(
			modal.getByText("Timeout must be a whole number from 1 to 300 seconds.", { exact: true }),
		).toBeVisible();
		await modal.getByLabel("Timeout (seconds)", { exact: true }).fill("45");
		await modal.getByRole("button", { name: "Add connection", exact: true }).click();

		const accountCard = cardForHeading(page, "Work CalDAV");
		await expect(accountCard).toBeVisible();
		await expect(accountCard.getByText("https://cal.example.test/dav", { exact: true })).toBeVisible();
		await expect(accountCard.getByText("Password configured", { exact: true })).toBeVisible();
		expect(await page.evaluate((secret) => document.documentElement.innerHTML.includes(secret), password)).toBe(false);

		await accountCard.getByRole("button", { name: "Test connection", exact: true }).click();
		modal = page.locator(".modal-box").filter({
			has: page.getByRole("heading", { name: "Discovered calendars", exact: true }),
		});
		await expect(modal.getByText("Work", { exact: true })).toBeVisible();
		await expect(modal.getByText("Personal", { exact: true })).toBeVisible();
		await expect(modal.getByText("Sync-token capable", { exact: true })).toBeVisible();
		await modal.getByRole("button", { name: "Close", exact: true }).click();

		await page.getByRole("tab", { name: "Datasets", exact: true }).click();
		await page.getByRole("button", { name: "Add dataset", exact: true }).click();
		modal = page.locator(".modal-box").filter({
			has: page.getByRole("heading", { name: "Add CalDAV dataset", exact: true }),
		});
		await expect(modal.getByRole("button", { name: /Work CalDAV/ })).toHaveAttribute("aria-pressed", "true");
		const instruction =
			"Keep accepted events from every work calendar between August and December, sync hourly, and export JSONL and Markdown.";
		const revisedInstruction = `${instruction} Include calendars added later.`;
		const saveDataset = modal.getByRole("button", { name: "Create dataset", exact: true });
		await expect(saveDataset).toBeDisabled();
		await modal.getByLabel("What should Moltis keep locally?", { exact: true }).fill(instruction);
		await expect(modal.getByText("Examples", { exact: true })).toBeVisible();
		await modal.getByText("Advanced dataset JSON", { exact: true }).click();
		const overrides = modal.getByLabel("Dataset overrides (JSON)", { exact: true });
		await overrides.fill("[]");
		await modal.getByRole("button", { name: "Compile", exact: true }).click();
		await expect(modal.getByText("Advanced dataset JSON must be a valid JSON object.", { exact: true })).toBeVisible();
		expect(
			await page.evaluate(
				() =>
					window.__connectorRpcState.requests.filter((request) => request.method === "connectors.datasets.compile")
						.length,
			),
		).toBe(0);

		await overrides.fill('{"scheduleMinutes":60}');
		await modal.getByRole("button", { name: "Compile", exact: true }).click();
		const compiledPreview = modal.getByLabel("Compiled dataset preview", { exact: true });
		await expect(compiledPreview).toContainText("Accepted work events will be kept locally");
		await expect(compiledPreview).toContainText("Calendar archive");
		await expect(compiledPreview).toContainText("All calendars");
		await expect(compiledPreview).toContainText("2026-08-01 until before 2026-12-31");
		await expect(compiledPreview).toContainText("Accepted by this account only");
		await expect(compiledPreview).toContainText("Every 60 minutes");
		await expect(compiledPreview).toContainText("JSONL, Markdown");
		await expect(compiledPreview).toContainText("Enabled");
		await expect(compiledPreview).toContainText("New calendars added later will also be included.");
		await expect(saveDataset).toBeEnabled();
		const firstCompileRequest = await page.evaluate(() =>
			window.__connectorRpcState.requests.find((request) => request.method === "connectors.datasets.compile"),
		);
		expect(firstCompileRequest.params).toEqual({
			accountId: "account-1",
			instruction,
			overrides: { scheduleMinutes: 60 },
		});

		await modal.getByLabel("What should Moltis keep locally?", { exact: true }).fill(revisedInstruction);
		await expect(saveDataset).toBeDisabled();
		await expect(
			compiledPreview.getByText("Compile the current instruction and overrides before saving.", { exact: true }),
		).toBeVisible();
		await modal.getByRole("button", { name: "Compile", exact: true }).click();
		await expect(saveDataset).toBeEnabled();
		await overrides.fill('{"scheduleMinutes":90}');
		await expect(saveDataset).toBeDisabled();
		await modal.getByRole("button", { name: "Compile", exact: true }).click();
		await expect(compiledPreview).toContainText("Every 90 minutes");
		await modal.getByRole("button", { name: "Create dataset", exact: true }).click();

		const datasetCard = cardForHeading(page, "Calendar archive");
		await expect(datasetCard).toContainText("Every 90 minutes");
		await expect(datasetCard.getByText("Needs sync", { exact: true })).toBeVisible();
		const addDatasetRequest = await page.evaluate(() =>
			window.__connectorRpcState.requests.find((request) => request.method === "connectors.datasets.add"),
		);
		expect(addDatasetRequest.params).toEqual({
			accountId: "account-1",
			instruction: revisedInstruction,
			name: "Calendar archive",
			config: {
				schemaVersion: 1,
				selection: { mode: "all" },
				filters: { startDate: "2026-08-01", endDate: "2026-12-31", acceptedByAccount: true },
			},
			scheduleMinutes: 90,
			projections: { jsonl: true, markdown: true },
			enabled: true,
		});

		await datasetCard.getByRole("button", { name: "Edit", exact: true }).click();
		modal = page.locator(".modal-box").filter({
			has: page.getByRole("heading", { name: "Edit CalDAV dataset", exact: true }),
		});
		await expect(modal.getByLabel("What should Moltis keep locally?", { exact: true })).toHaveValue(revisedInstruction);
		await expect(modal.getByText("Current saved dataset configuration.", { exact: true })).toBeVisible();
		await expect(modal.getByRole("button", { name: "Save dataset", exact: true })).toBeDisabled();
		await modal.getByRole("button", { name: "Compile", exact: true }).click();
		await modal.getByRole("button", { name: "Save dataset", exact: true }).click();
		const editCompileRequest = await page.evaluate(() =>
			window.__connectorRpcState.requests.findLast((request) => request.method === "connectors.datasets.compile"),
		);
		expect(editCompileRequest.params).toEqual({
			accountId: "account-1",
			datasetId: "dataset-1",
			instruction: revisedInstruction,
		});
		const updateDatasetRequest = await page.evaluate(() =>
			window.__connectorRpcState.requests.findLast((request) => request.method === "connectors.datasets.update"),
		);
		expect(updateDatasetRequest.params.instruction).toBe(revisedInstruction);

		await datasetCard.getByRole("button", { name: "Run now", exact: true }).click();
		await expect(datasetCard.getByText("1 items", { exact: true })).toBeVisible();
		await expect(datasetCard.getByText("Needs sync", { exact: true })).toHaveCount(0);
		await datasetCard.getByRole("button", { name: "Preview", exact: true }).click();
		modal = page.locator(".modal-box").filter({
			has: page.getByRole("heading", { name: "Dataset preview", exact: true }),
		});
		await expect(modal.locator("pre")).toContainText("Planning review");
		const itemQuery = await page.evaluate(() =>
			window.__connectorRpcState.requests.find((request) => request.method === "connectors.items.query"),
		);
		expect(itemQuery.params).toEqual({ datasetId: "dataset-1", limit: 25, offset: 0, includeDeleted: false });
		await modal.getByRole("button", { name: "Close", exact: true }).click();

		await page.getByRole("tab", { name: "Activity", exact: true }).click();
		await expect(page.getByLabel("Dataset", { exact: true })).toHaveValue("dataset-1");
		await expect(page.getByText("Succeeded", { exact: true })).toBeVisible();
		await expect(page.getByText("1 upserted, 0 deleted, 1 active", { exact: true })).toBeVisible();

		await page.getByRole("tab", { name: "Connections", exact: true }).click();
		const renamedAccountCard = cardForHeading(page, "Work CalDAV");
		await renamedAccountCard.getByRole("button", { name: "Edit", exact: true }).click();
		modal = page.locator(".modal-box").filter({
			has: page.getByRole("heading", { name: "Edit CalDAV connection", exact: true }),
		});
		await expect(modal.getByLabel("Password", { exact: true })).toHaveValue("");
		await modal.getByLabel("Connection name", { exact: true }).fill("Work CalDAV updated");
		await modal.getByRole("button", { name: "Save connection", exact: true }).click();
		await expect(cardForHeading(page, "Work CalDAV updated")).toBeVisible();
		const updateRequest = await page.evaluate(() =>
			window.__connectorRpcState.requests.findLast((request) => request.method === "connectors.accounts.update"),
		);
		expect(updateRequest.params).not.toHaveProperty("password");

		await page.getByRole("tab", { name: "Datasets", exact: true }).click();
		await cardForHeading(page, "Calendar archive").getByRole("button", { name: "Remove", exact: true }).click();
		modal = page.locator(".modal-box").filter({
			has: page.getByText("Remove dataset Calendar archive and its synchronized items?", { exact: true }),
		});
		await modal.getByRole("button", { name: "Remove", exact: true }).click();
		await expect(page.getByText("No connector datasets configured.", { exact: true })).toBeVisible();

		await page.getByRole("tab", { name: "Connections", exact: true }).click();
		await cardForHeading(page, "Work CalDAV updated").getByRole("button", { name: "Remove", exact: true }).click();
		modal = page.locator(".modal-box").filter({
			has: page.getByText(
				"Remove connection Work CalDAV updated? Its datasets and synchronized items will also be removed.",
				{ exact: true },
			),
		});
		await modal.getByRole("button", { name: "Remove", exact: true }).click();
		await expect(page.getByText("No connector accounts configured.", { exact: true })).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("protects config-managed connections and disabled datasets", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/connectors");
		await waitForWsConnected(page);
		await installConnectorsRpcMock(page);
		await page.evaluate(() => {
			const now = "2026-08-06T10:00:00Z";
			window.__connectorRpcState.accounts = [
				{
					id: "managed-account",
					kind: "caldav",
					name: "Configured calendar",
					serverUrl: "https://calendar.example.test",
					username: "managed@example.test",
					timeoutSeconds: 45,
					allowInsecureHttp: false,
					allowPrivateNetwork: false,
					hasPassword: false,
					managed: true,
					enabled: false,
					createdAt: now,
					updatedAt: now,
				},
			];
			window.__connectorRpcState.datasets = [
				{
					id: "managed-dataset",
					accountId: "managed-account",
					name: "Configured archive",
					instruction: "Keep all calendars",
					config: { schemaVersion: 1, selection: { mode: "all" }, filters: { acceptedByAccount: false } },
					projections: { jsonl: false, markdown: false },
					enabled: true,
					itemCount: 0,
					needsSync: true,
					createdAt: now,
					updatedAt: now,
				},
			];
		});
		await page.getByRole("button", { name: "Refresh", exact: true }).click();

		const accountCard = cardForHeading(page, "Configured calendar");
		await expect(accountCard.getByText("Managed by moltis.toml", { exact: true })).toBeVisible();
		await expect(accountCard.getByRole("button", { name: "Test connection", exact: true })).toBeDisabled();
		await expect(accountCard.getByRole("button", { name: "Remove", exact: true })).toBeDisabled();
		await accountCard.getByRole("button", { name: "Edit", exact: true }).click();
		const modal = page.locator(".modal-box").filter({
			has: page.getByRole("heading", { name: "Edit CalDAV connection", exact: true }),
		});
		await expect(modal.getByText(/identity and credentials come from moltis\.toml/)).toBeVisible();
		await expect(modal.getByLabel("Connection name", { exact: true })).toBeDisabled();
		await expect(modal.getByLabel("Server URL", { exact: true })).toBeDisabled();
		await expect(modal.getByLabel("Username", { exact: true })).toBeDisabled();
		await expect(modal.getByLabel("Password", { exact: true })).toBeDisabled();
		await expect(modal.getByLabel("Enabled", { exact: true })).toBeDisabled();
		await modal.getByText("Advanced network settings", { exact: true }).click();
		await expect(modal.getByLabel("Timeout (seconds)", { exact: true })).toBeDisabled();
		await modal.getByLabel("Allow private network addresses", { exact: true }).check();
		await modal.getByRole("button", { name: "Save connection", exact: true }).click();
		const updateRequest = await page.evaluate(() =>
			window.__connectorRpcState.requests.findLast((request) => request.method === "connectors.accounts.update"),
		);
		expect(updateRequest.params.allowPrivateNetwork).toBe(true);
		expect(updateRequest.params).not.toHaveProperty("password");

		await page.getByRole("tab", { name: "Datasets", exact: true }).click();
		const datasetCard = cardForHeading(page, "Configured archive");
		await expect(datasetCard.getByRole("button", { name: "Run now", exact: true })).toBeDisabled();
		expect(pageErrors).toEqual([]);
	});

	test("command palette finds the connectors settings entry by CalDAV keyword", async ({ page }) => {
		await navigateAndWait(page, "/settings/profile");
		await page.keyboard.press("Control+k");
		await page.locator(".cmd-palette-input").fill("caldav");
		const item = page.locator(".cmd-palette-item").filter({ hasText: "Connectors" });
		await expect(item).toBeVisible();
		await item.click();
		await expect(page).toHaveURL(/\/settings\/connectors$/);
		await expect(page.getByRole("heading", { name: "Connectors", exact: true })).toBeVisible();
	});
});
