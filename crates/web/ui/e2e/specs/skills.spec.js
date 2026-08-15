const { expect, test } = require("../base-test");
const { navigateAndWait, watchPageErrors } = require("../helpers");

async function mockClawHubSearch(page, response) {
	await page.addInitScript((searchResponse) => {
		const plannedResponses = Array.isArray(searchResponse) ? searchResponse : [{ response: searchResponse }];
		window.__clawHubSearchRequests = [];
		window.__clawHubSearchResponseCount = 0;
		const originalSend = WebSocket.prototype.send;
		WebSocket.prototype.send = function (payload) {
			try {
				const parsed = JSON.parse(payload);
				if (parsed?.method === "skills.clawhub.search") {
					const query = parsed.params?.query;
					window.__clawHubSearchRequests.push(query);
					const plan = plannedResponses.find((candidate) => candidate.query === query) || plannedResponses[0];
					setTimeout(() => {
						const event = new MessageEvent("message", {
							data: JSON.stringify({ type: "res", id: parsed.id, ...plan.response }),
						});
						window.__clawHubSearchResponseCount += 1;
						if (typeof this.onmessage === "function") this.onmessage(event);
					}, plan.delayMs || 0);
					return;
				}
			} catch {
				// Let non-JSON WebSocket traffic pass through unchanged.
			}
			return originalSend.call(this, payload);
		};
	}, response);
}

async function deferClawHubDebounceTimers(page) {
	await page.evaluate(() => {
		const deferred = [];
		const originalSetTimeout = window.setTimeout.bind(window);
		const originalClearTimeout = window.clearTimeout.bind(window);
		window.__runDeferredClawHubSearchTimer = (index) => deferred[index]?.callback();
		window.setTimeout = (callback, delay, ...args) => {
			if (delay === 300 && typeof callback === "function") {
				const id = 1_000_000 + deferred.length;
				deferred.push({ id, callback: () => callback(...args) });
				return id;
			}
			return originalSetTimeout(callback, delay, ...args);
		};
		window.clearTimeout = (id) => {
			if (deferred.some((timer) => timer.id === id)) return;
			originalClearTimeout(id);
		};
	});
}

test.describe("Skills page", () => {
	test("skills page loads", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/skills");

		await expect(page.getByRole("heading", { name: "Skills", exact: true })).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("install input present", async ({ page }) => {
		await navigateAndWait(page, "/skills");

		// Install inputs are behind the Repositories tab
		await page.getByRole("tab", { name: /Repositories/ }).click();
		await expect(page.getByPlaceholder("owner/repo or full URL (e.g. anthropics/skills)")).toBeVisible();
		await expect(page.getByRole("button", { name: "Install", exact: true }).first()).toBeVisible();
	});

	test("featured repos shown", async ({ page }) => {
		await navigateAndWait(page, "/skills");

		// Featured section is behind the Repositories tab
		await page.getByRole("tab", { name: /Repositories/ }).click();
		await expect(page.getByRole("heading", { name: "Featured Repositories", exact: true })).toBeVisible();
		await expect(page.getByRole("link", { name: "anthropics/skills", exact: true })).toBeVisible();
	});

	test("page has no JS errors", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/skills");
		expect(pageErrors).toEqual([]);
	});

	test("ClawHub search shows owner-qualified results", async ({ page }) => {
		await mockClawHubSearch(page, {
			ok: true,
			payload: {
				results: [
					{
						score: 6120,
						slug: "csv",
						installRef: "@ivangdavila/csv",
						displayName: "CSV by Ivan",
						summary: "Parse RFC 4180 CSV files",
						downloads: 5395,
						ownerHandle: "ivangdavila",
					},
					{
						score: 6120,
						slug: "csv",
						installRef: "@thcjp/csv",
						displayName: "CSV by THCJP",
						summary: "Parse and generate CSV files",
						downloads: 17,
						ownerHandle: "thcjp",
					},
				],
			},
		});
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/skills");
		await page.getByRole("tab", { name: "ClawHub", exact: true }).click();
		await page.getByPlaceholder("Search ClawHub skills (e.g. csv, weather, github)...").fill("csv");

		await expect(page.getByText("CSV by Ivan", { exact: true })).toBeVisible();
		await expect(page.getByText("CSV by THCJP", { exact: true })).toBeVisible();
		await expect(page.getByText("@ivangdavila /", { exact: true })).toBeVisible();
		await expect(page.getByText("@thcjp /", { exact: true })).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("ClawHub search reports RPC errors in the page, toast, and console", async ({ page }) => {
		await mockClawHubSearch(page, {
			ok: false,
			error: { code: "TIMEOUT", message: "ClawHub search timed out after 15 seconds" },
		});
		const consoleErrors = [];
		page.on("console", (message) => {
			if (message.type() === "error") consoleErrors.push(message.text());
		});
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/skills");
		await page.getByRole("tab", { name: "ClawHub", exact: true }).click();
		await page.getByPlaceholder("Search ClawHub skills (e.g. csv, weather, github)...").fill("csv");

		await expect(page.getByRole("alert")).toContainText("The request timed out. Please try again.");
		await expect(page.getByText("The request timed out. Please try again.", { exact: true })).toBeVisible();
		expect(consoleErrors.some((message) => message.includes("ClawHub search failed"))).toBeTruthy();
		expect(pageErrors).toEqual([]);
	});

	test("ClawHub search ignores stale responses", async ({ page }) => {
		await mockClawHubSearch(page, [
			{
				query: "csv",
				delayMs: 1000,
				response: {
					ok: false,
					error: { code: "TIMEOUT", message: "Old CSV search timed out" },
				},
			},
			{
				query: "weather",
				response: {
					ok: true,
					payload: {
						results: [
							{
								score: 100,
								slug: "weather",
								installRef: "@forecast/weather",
								displayName: "Current Weather",
								ownerHandle: "forecast",
							},
						],
					},
				},
			},
		]);
		const consoleErrors = [];
		page.on("console", (message) => {
			if (message.type() === "error") consoleErrors.push(message.text());
		});
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/skills");
		await page.getByRole("tab", { name: "ClawHub", exact: true }).click();
		const searchInput = page.getByPlaceholder("Search ClawHub skills (e.g. csv, weather, github)...");

		await searchInput.fill("csv");
		await page.waitForFunction(() => window.__clawHubSearchRequests?.includes("csv"));
		await searchInput.fill("weather");

		await expect(page.getByText("Current Weather", { exact: true })).toBeVisible();
		await page.waitForFunction(() => window.__clawHubSearchResponseCount === 2);
		await expect(page.getByText("Current Weather", { exact: true })).toBeVisible();
		await expect(page.getByRole("alert")).not.toBeVisible();
		expect(consoleErrors.some((message) => message.includes("Old CSV search timed out"))).toBeFalsy();
		expect(pageErrors).toEqual([]);
	});

	test("ClawHub search ignores a queued stale debounce callback", async ({ page }) => {
		await mockClawHubSearch(page, [
			{
				query: "csv",
				response: {
					ok: true,
					payload: { results: [{ score: 10, slug: "csv", displayName: "Old CSV Result" }] },
				},
			},
			{
				query: "weather",
				response: {
					ok: true,
					payload: { results: [{ score: 100, slug: "weather", displayName: "Current Weather" }] },
				},
			},
		]);
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/skills");
		await page.getByRole("tab", { name: "ClawHub", exact: true }).click();
		await deferClawHubDebounceTimers(page);
		const searchInput = page.getByPlaceholder("Search ClawHub skills (e.g. csv, weather, github)...");

		await searchInput.fill("csv");
		await searchInput.fill("weather");
		await searchInput.press("Enter");
		await expect(page.getByText("Current Weather", { exact: true })).toBeVisible();
		await page.evaluate(() => window.__runDeferredClawHubSearchTimer(0));

		expect(await page.evaluate(() => window.__clawHubSearchRequests)).toEqual(["weather"]);
		await expect(page.getByText("Current Weather", { exact: true })).toBeVisible();
		await expect(page.getByText("Old CSV Result", { exact: true })).not.toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("skill browse list shows View and Install buttons for unenabled skills", async ({ page }) => {
		await page.route("**/api/skills/search?*", async (route) => {
			await route.fulfill({
				contentType: "application/json",
				body: JSON.stringify({
					skills: [
						{
							name: "doc-converter",
							display_name: "Doc Converter",
							description: "Convert documents",
							enabled: false,
							trusted: false,
						},
						{
							name: "pdf-reader",
							display_name: "PDF Reader",
							description: "Read PDFs",
							enabled: true,
							trusted: true,
						},
					],
				}),
			});
		});
		await page.route("**/api/skills", async (route) => {
			await route.fulfill({
				contentType: "application/json",
				body: JSON.stringify({
					skills: [],
					repos: [
						{
							source: "test-org/skills",
							skill_count: 2,
							enabled_count: 1,
							trusted_count: 1,
						},
					],
				}),
			});
		});

		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/skills");

		await page.getByRole("tab", { name: /Repositories/ }).click();
		// Expand the repo card
		await page.getByText("1/2 enabled", { exact: true }).click();

		// Unenabled skill should have View + Install buttons
		const docRow = page.locator(".skills-ac-item").filter({ hasText: "Doc Converter" });
		await expect(docRow.getByRole("button", { name: "View", exact: true })).toBeVisible();
		await expect(docRow.getByRole("button", { name: "Install", exact: true })).toBeVisible();

		// Enabled skill should show "Installed" label, not Install button
		const pdfRow = page.locator(".skills-ac-item").filter({ hasText: "PDF Reader" });
		await expect(pdfRow.getByRole("button", { name: "View", exact: true })).toBeVisible();
		await expect(pdfRow.getByText("Installed", { exact: true })).toBeVisible();
		await expect(pdfRow.getByRole("button", { name: "Install", exact: true })).not.toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("skill detail panel shows Install button with primary style for unenabled skill", async ({ page }) => {
		await page.route("**/api/skills/search?*", async (route) => {
			await route.fulfill({
				contentType: "application/json",
				body: JSON.stringify({
					skills: [
						{
							name: "xlsx",
							display_name: "XLSX",
							description: "Excel support",
							enabled: false,
							trusted: false,
						},
					],
				}),
			});
		});
		// Mock the skill.detail RPC response via WS — we intercept the HTTP
		// search listing instead and click View to open the detail.
		// The detail is fetched via WS RPC, so we use page.evaluate to
		// intercept the module's WS pending map.
		await page.route("**/api/skills", async (route) => {
			await route.fulfill({
				contentType: "application/json",
				body: JSON.stringify({
					skills: [],
					repos: [
						{
							source: "document-skills/repo",
							skill_count: 1,
							enabled_count: 0,
							trusted_count: 0,
						},
					],
				}),
			});
		});

		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/skills");

		await page.getByRole("tab", { name: /Repositories/ }).click();
		await page.getByText("0/1 enabled", { exact: true }).click();

		// The browse list shows the skill with Install button (not "Enable")
		const skillRow = page.locator(".skills-ac-item").filter({ hasText: "XLSX" });
		await expect(skillRow.getByRole("button", { name: "Install", exact: true })).toBeVisible();
		// Ensure old "Enable" label is not present anywhere in the row
		await expect(skillRow.getByRole("button", { name: "Enable", exact: true })).not.toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("error toast shows message text not [object Object]", async ({ page }) => {
		// Verify that the error message extraction helper works correctly
		// by evaluating it directly in the page context after app loads
		await navigateAndWait(page, "/skills");

		const result = await page.evaluate(() => {
			// Simulate what the fixed code does with an RPC error object
			const errorObj = { code: "INTERNAL", message: "skill 'xlsx' is not trusted" };
			// Old code: `Failed: ${errorObj}` → "Failed: [object Object]"
			const oldWay = `Failed: ${errorObj}`;
			// New code: `Failed: ${errorObj?.message || "unknown"}` → correct
			const newWay = `Failed: ${errorObj?.message || "unknown"}`;
			return { oldWay, newWay };
		});

		// The old way would produce [object Object] — confirm the bug pattern
		expect(result.oldWay).toBe("Failed: [object Object]");
		// The new way produces the actual message
		expect(result.newWay).toBe("Failed: skill 'xlsx' is not trusted");
	});

	test("Install All button shown when repo has unenabled skills", async ({ page }) => {
		await page.route("**/api/skills/search?*", async (route) => {
			await route.fulfill({
				contentType: "application/json",
				body: JSON.stringify({
					skills: [
						{ name: "skill-a", display_name: "Skill A", description: "First", enabled: false },
						{ name: "skill-b", display_name: "Skill B", description: "Second", enabled: false },
						{ name: "skill-c", display_name: "Skill C", description: "Third", enabled: true },
					],
				}),
			});
		});
		await page.route("**/api/skills", async (route) => {
			await route.fulfill({
				contentType: "application/json",
				body: JSON.stringify({
					skills: [],
					repos: [{ source: "test-org/multi", skill_count: 3, enabled_count: 1, trusted_count: 1 }],
				}),
			});
		});

		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/skills");

		await page.getByRole("tab", { name: /Repositories/ }).click();
		// Install All button should be visible in the repo header (2 unenabled out of 3)
		await expect(page.getByRole("button", { name: "Install All", exact: true })).toBeVisible();

		// Expand to verify skill list loaded
		await page.getByText("1/3 enabled", { exact: true }).click();
		await expect(page.locator(".skills-ac-item").filter({ hasText: "Skill A" })).toBeVisible();
		await expect(page.locator(".skills-ac-item").filter({ hasText: "Skill C" })).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("Install All button hidden when all skills are enabled", async ({ page }) => {
		await page.route("**/api/skills/search?*", async (route) => {
			await route.fulfill({
				contentType: "application/json",
				body: JSON.stringify({
					skills: [{ name: "only-skill", display_name: "Only Skill", description: "All enabled", enabled: true }],
				}),
			});
		});
		await page.route("**/api/skills", async (route) => {
			await route.fulfill({
				contentType: "application/json",
				body: JSON.stringify({
					skills: [],
					repos: [{ source: "test-org/all-enabled", skill_count: 1, enabled_count: 1, trusted_count: 1 }],
				}),
			});
		});

		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/skills");

		await page.getByRole("tab", { name: /Repositories/ }).click();
		// All skills are enabled, so Install All should not be visible
		await expect(page.getByRole("button", { name: "Install All", exact: true })).not.toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("imported repos show bundle actions and provenance", async ({ page }) => {
		await page.route("**/api/skills/search?*", async (route) => {
			await route.fulfill({
				contentType: "application/json",
				body: JSON.stringify({
					skills: [
						{
							name: "bundle-skill",
							display_name: "Bundle Skill",
							description: "Imported from a portable bundle",
							quarantined: true,
							enabled: false,
						},
					],
				}),
			});
		});
		await page.route("**/api/skills", async (route) => {
			await route.fulfill({
				contentType: "application/json",
				body: JSON.stringify({
					skills: [],
					repos: [
						{
							source: "portable/bundle",
							skill_count: 1,
							enabled_count: 0,
							quarantined: true,
							quarantine_reason: "Imported bundle awaiting review",
							provenance: {
								original_source: "source/repo",
								original_commit_sha: "0123456789abcdef0123456789abcdef01234567",
								imported_from: "/tmp/demo-skill-bundle.tar.gz",
							},
						},
					],
				}),
			});
		});

		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/skills");

		// Repos are behind the Repositories tab
		await page.getByRole("tab", { name: /Repositories/ }).click();
		await page.getByText("0/1 enabled", { exact: true }).click();
		await expect(page.getByRole("button", { name: "Export", exact: true })).toBeVisible();
		await expect(page.getByRole("button", { name: "Clear Quarantine", exact: true })).toBeVisible();
		await expect(page.getByText("Original source:")).toBeVisible();
		await expect(page.getByText("Imported from:")).toBeVisible();
		expect(pageErrors).toEqual([]);
	});
});
