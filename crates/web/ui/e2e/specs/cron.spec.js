const { expect, test } = require("@playwright/test");
const { navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

function isRetryableRpcError(message) {
	if (typeof message !== "string") return false;
	return message.includes("WebSocket not connected") || message.includes("WebSocket disconnected");
}

async function sendRpcFromPage(page, method, params) {
	let lastResponse = null;
	for (let attempt = 0; attempt < 40; attempt++) {
		if (attempt > 0) {
			await waitForWsConnected(page);
			await page.waitForTimeout(100);
		}
		lastResponse = await page
			.evaluate(
				async ({ methodName, methodParams }) => {
					var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
					if (!appScript) throw new Error("app module script not found");
					var appUrl = new URL(appScript.src, window.location.origin);
					var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
					var helpers = await import(`${prefix}js/helpers.js`);
					return helpers.sendRpc(methodName, methodParams);
				},
				{
					methodName: method,
					methodParams: params,
				},
			)
			.catch((error) => ({ ok: false, error: { message: error?.message || String(error) } }));

		if (lastResponse?.ok) return lastResponse;
		if (!isRetryableRpcError(lastResponse?.error?.message)) return lastResponse;
	}

	return lastResponse;
}

async function expectRpcOk(page, method, params) {
	const response = await sendRpcFromPage(page, method, params);
	expect(response?.ok, `RPC ${method} failed: ${response?.error?.message || "unknown error"}`).toBeTruthy();
	return response;
}

test.describe("Cron jobs page", () => {
	test("cron page loads with heading", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/crons");

		await expect(page.getByRole("heading", { name: "Cron Jobs", exact: true })).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("heartbeat tab loads", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/heartbeat");

		await expect(page.getByRole("heading", { name: /heartbeat/i })).toBeVisible();
		await expect(page.getByText("Deliver to channel", { exact: true })).toBeVisible();
		await expect(page.getByText("Channel Account", { exact: true })).toBeVisible();
		await expect(page.getByText("Chat ID", { exact: true })).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("heartbeat inactive state disables run now with info notice", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/heartbeat");

		await expect(page.getByRole("button", { name: "Run Now", exact: true })).toBeDisabled();
		await expect(page.getByText(/Heartbeat inactive:/)).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("create job button present", async ({ page }) => {
		await navigateAndWait(page, "/settings/crons");

		// Page should have content, create button may depend on state
		const content = page.locator("#pageContent");
		await expect(content).not.toBeEmpty();
	});

	test("cron modal exposes model and execution controls", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/crons");

		await page.getByRole("button", { name: "+ Add Job", exact: true }).click();

		await expect(page.getByText("Model (Agent Turn)", { exact: true })).toBeVisible();
		await expect(page.getByText("Execution Target", { exact: true })).toBeVisible();
		await expect(page.getByText("Sandbox Image", { exact: true })).toBeVisible();

		await page.locator('[data-field="executionTarget"]').selectOption("host");
		await expect(page.locator('[data-field="executionTarget"]')).toHaveValue("host");
		expect(pageErrors).toEqual([]);
	});

	test("modal defaults are compatible: systemEvent + main", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/crons");

		await page.getByRole("button", { name: "+ Add Job", exact: true }).click();

		await expect(page.locator('[data-field="payloadKind"]')).toHaveValue("systemEvent");
		await expect(page.locator('[data-field="target"]')).toHaveValue("main");
		expect(pageErrors).toEqual([]);
	});

	test("auto-sync: switching payload kind updates session target", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/crons");

		await page.getByRole("button", { name: "+ Add Job", exact: true }).click();

		// Default state
		await expect(page.locator('[data-field="payloadKind"]')).toHaveValue("systemEvent");
		await expect(page.locator('[data-field="target"]')).toHaveValue("main");

		// Switch to agentTurn => target should become isolated
		await page.locator('[data-field="payloadKind"]').selectOption("agentTurn");
		await expect(page.locator('[data-field="target"]')).toHaveValue("isolated");

		// Switch back to systemEvent => target should become main
		await page.locator('[data-field="payloadKind"]').selectOption("systemEvent");
		await expect(page.locator('[data-field="target"]')).toHaveValue("main");

		// Switch target to isolated => payload should become agentTurn
		await page.locator('[data-field="target"]').selectOption("isolated");
		await expect(page.locator('[data-field="payloadKind"]')).toHaveValue("agentTurn");

		// Switch target to main => payload should become systemEvent
		await page.locator('[data-field="target"]').selectOption("main");
		await expect(page.locator('[data-field="payloadKind"]')).toHaveValue("systemEvent");

		expect(pageErrors).toEqual([]);
	});

	test("form fields survive schedule type change", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/crons");

		await page.getByRole("button", { name: "+ Add Job", exact: true }).click();

		// Fill in the name field
		await page.locator('[data-field="name"]').fill("test-job-persist");
		await expect(page.locator('[data-field="name"]')).toHaveValue("test-job-persist");

		// Change schedule type from cron to every
		await page.locator('[data-field="schedKind"]').selectOption("every");

		// Name should still be there
		await expect(page.locator('[data-field="name"]')).toHaveValue("test-job-persist");

		// Change schedule type again to at
		await page.locator('[data-field="schedKind"]').selectOption("at");
		await expect(page.locator('[data-field="name"]')).toHaveValue("test-job-persist");

		expect(pageErrors).toEqual([]);
	});

	test("delivery toggle visible only for agentTurn", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/crons");

		await page.getByRole("button", { name: "+ Add Job", exact: true }).click();

		// Default: systemEvent — no delivery toggle
		await expect(page.getByText("Deliver output to channel")).not.toBeVisible();

		// Switch to agentTurn — delivery toggle should appear
		await page.locator('[data-field="payloadKind"]').selectOption("agentTurn");
		await expect(page.getByText("Deliver output to channel")).toBeVisible();

		// Switch back to systemEvent — delivery toggle should hide
		await page.locator('[data-field="payloadKind"]').selectOption("systemEvent");
		await expect(page.getByText("Deliver output to channel")).not.toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("delivery toggle shows and hides channel fields", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/crons");

		await page.getByRole("button", { name: "+ Add Job", exact: true }).click();
		await page.locator('[data-field="payloadKind"]').selectOption("agentTurn");

		// Channel fields not visible before checking toggle
		await expect(page.getByText("Channel Account", { exact: true })).not.toBeVisible();
		await expect(page.getByText("Chat ID (recipient)", { exact: true })).not.toBeVisible();

		// Check the delivery toggle
		const toggle = page.getByText("Deliver output to channel");
		await toggle.scrollIntoViewIfNeeded();
		await toggle.click();

		// Channel fields should appear (scroll to make visible)
		const chatIdLabel = page.getByText("Chat ID (recipient)", { exact: true });
		await chatIdLabel.scrollIntoViewIfNeeded();
		await expect(page.getByText("Channel Account", { exact: true })).toBeVisible();
		await expect(chatIdLabel).toBeVisible();

		// Uncheck the toggle
		await toggle.scrollIntoViewIfNeeded();
		await toggle.click();

		// Channel fields should disappear
		await expect(page.getByText("Channel Account", { exact: true })).not.toBeVisible();
		await expect(page.getByText("Chat ID (recipient)", { exact: true })).not.toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("edit modal populates fields from existing systemEvent job", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/crons");
		await waitForWsConnected(page);

		// Create a systemEvent job via RPC
		const addRes = await expectRpcOk(page, "cron.add", {
			name: "e2e-edit-sys",
			schedule: { kind: "every", every_ms: 60000 },
			payload: { kind: "systemEvent", text: "hello from e2e" },
			sessionTarget: "main",
			enabled: false,
			sandbox: { enabled: true },
		});
		const jobId = addRes.payload?.id;

		// Reload the page so the new job appears in the table
		await navigateAndWait(page, "/settings/crons");
		await waitForWsConnected(page);

		// Click Edit on the created job row
		const row = page.locator("tr", { hasText: "e2e-edit-sys" });
		await expect(row).toBeVisible();
		await row.getByRole("button", { name: "Edit", exact: true }).click();

		// Assert modal fields match the job values
		await expect(page.locator('[data-field="name"]')).toHaveValue("e2e-edit-sys");
		await expect(page.locator('[data-field="schedKind"]')).toHaveValue("every");
		await expect(page.locator('[data-field="payloadKind"]')).toHaveValue("systemEvent");
		await expect(page.locator('[data-field="target"]')).toHaveValue("main");
		await expect(page.locator('[data-field="message"]')).toHaveValue("hello from e2e");

		// Clean up
		if (jobId) await sendRpcFromPage(page, "cron.remove", { id: jobId });
		expect(pageErrors).toEqual([]);
	});

	test("edit modal populates fields from existing agentTurn job", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/crons");
		await waitForWsConnected(page);

		// Create an agentTurn job via RPC
		const addRes = await expectRpcOk(page, "cron.add", {
			name: "e2e-edit-agent",
			schedule: { kind: "cron", expr: "0 * * * *" },
			payload: { kind: "agentTurn", message: "agent prompt text" },
			sessionTarget: "isolated",
			enabled: true,
			sandbox: { enabled: false },
		});
		const jobId = addRes.payload?.id;

		await navigateAndWait(page, "/settings/crons");
		await waitForWsConnected(page);

		const row = page.locator("tr", { hasText: "e2e-edit-agent" });
		await expect(row).toBeVisible();
		await row.getByRole("button", { name: "Edit", exact: true }).click();

		await expect(page.locator('[data-field="name"]')).toHaveValue("e2e-edit-agent");
		await expect(page.locator('[data-field="schedKind"]')).toHaveValue("cron");
		await expect(page.locator('[data-field="payloadKind"]')).toHaveValue("agentTurn");
		await expect(page.locator('[data-field="target"]')).toHaveValue("isolated");
		await expect(page.locator('[data-field="message"]')).toHaveValue("agent prompt text");
		await expect(page.locator('[data-field="executionTarget"]')).toHaveValue("host");

		// Clean up
		if (jobId) await sendRpcFromPage(page, "cron.remove", { id: jobId });
		expect(pageErrors).toEqual([]);
	});

	test("edit then add resets modal to defaults", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/crons");
		await waitForWsConnected(page);

		// Create a job to edit
		const addRes = await expectRpcOk(page, "cron.add", {
			name: "e2e-reset-check",
			schedule: { kind: "every", every_ms: 30000 },
			payload: { kind: "agentTurn", message: "custom prompt" },
			sessionTarget: "isolated",
			enabled: false,
			sandbox: { enabled: false },
		});
		const jobId = addRes.payload?.id;

		await navigateAndWait(page, "/settings/crons");
		await waitForWsConnected(page);

		// Open Edit modal
		const row = page.locator("tr", { hasText: "e2e-reset-check" });
		await expect(row).toBeVisible();
		await row.getByRole("button", { name: "Edit", exact: true }).click();

		// Verify it's populated
		await expect(page.locator('[data-field="name"]')).toHaveValue("e2e-reset-check");

		// Close modal
		await page.keyboard.press("Escape");

		// Open Add modal
		await page.getByRole("button", { name: "+ Add Job", exact: true }).click();

		// Fields should be reset to defaults
		await expect(page.locator('[data-field="name"]')).toHaveValue("");
		await expect(page.locator('[data-field="schedKind"]')).toHaveValue("cron");
		await expect(page.locator('[data-field="payloadKind"]')).toHaveValue("systemEvent");
		await expect(page.locator('[data-field="target"]')).toHaveValue("main");
		await expect(page.locator('[data-field="message"]')).toHaveValue("");

		// Clean up
		if (jobId) await sendRpcFromPage(page, "cron.remove", { id: jobId });
		expect(pageErrors).toEqual([]);
	});

	test("page has no JS errors", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/crons");
		expect(pageErrors).toEqual([]);
	});
});
