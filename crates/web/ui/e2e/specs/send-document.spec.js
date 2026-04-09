const { expect, test } = require("../base-test");
const { createSession, navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

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

async function openFreshChatSession(page) {
	await navigateAndWait(page, "/");
	await waitForWsConnected(page);
	await createSession(page);
	return page.evaluate(() => window.__moltis_stores?.sessionStore?.activeSessionKey?.value || "");
}

test.describe("send_document rendering", () => {
	test("renders document card with filename and download link for document_ref", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		const sessionKey = await openFreshChatSession(page);

		// Simulate tool_call_start to create the tool card
		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey,
				state: "tool_call_start",
				toolCallId: "test-doc-call",
				toolName: "send_document",
				arguments: JSON.stringify({ path: "/tmp/report.pdf" }),
			},
		});

		// Simulate tool_call_end with document_ref result
		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey,
				state: "tool_call_end",
				toolCallId: "test-doc-call",
				toolName: "send_document",
				success: true,
				result: {
					document_ref: "media/main/abc123_report.pdf",
					mime_type: "application/pdf",
					filename: "report.pdf",
					size_bytes: 12345,
				},
			},
		});

		// Verify the document card renders
		const docContainer = page.locator(".document-container").filter({ hasText: "report.pdf" });
		await expect(docContainer).toBeVisible({ timeout: 5_000 });

		// Verify filename is displayed
		const filenameEl = docContainer.locator(".document-filename");
		await expect(filenameEl).toHaveText("report.pdf");

		// Verify file size is displayed
		const sizeEl = docContainer.locator(".document-size");
		await expect(sizeEl).toHaveText("12.1 KB");

		// Verify download/open button exists and has correct href
		const downloadBtn = docContainer.locator(".document-download-btn");
		await expect(downloadBtn).toBeVisible();
		const href = await downloadBtn.getAttribute("href");
		expect(href).toContain(`/api/sessions/${encodeURIComponent(sessionKey)}/media/abc123_report.pdf`);

		// PDF should open in new tab (not trigger download)
		const target = await downloadBtn.getAttribute("target");
		expect(target).toBe("_blank");

		expect(pageErrors).toEqual([]);
	});

	test("renders document card for zip file with download attribute", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		const sessionKey = await openFreshChatSession(page);

		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey,
				state: "tool_call_start",
				toolCallId: "test-zip-call",
				toolName: "send_document",
				arguments: JSON.stringify({ path: "/tmp/archive.zip" }),
			},
		});

		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey,
				state: "tool_call_end",
				toolCallId: "test-zip-call",
				toolName: "send_document",
				success: true,
				result: {
					document_ref: "media/main/def456_archive.zip",
					mime_type: "application/zip",
					filename: "archive.zip",
					size_bytes: 5242880,
				},
			},
		});

		const docContainer = page.locator(".document-container").filter({ hasText: "archive.zip" });
		await expect(docContainer).toBeVisible({ timeout: 5_000 });

		const filenameEl = docContainer.locator(".document-filename");
		await expect(filenameEl).toHaveText("archive.zip");

		// Zip files should have a download attribute (not target=_blank)
		const downloadBtn = docContainer.locator(".document-download-btn");
		await expect(downloadBtn).toBeVisible();
		const downloadAttr = await downloadBtn.getAttribute("download");
		expect(downloadAttr).toBeTruthy();
		const target = await downloadBtn.getAttribute("target");
		expect(target).toBeNull();

		expect(pageErrors).toEqual([]);
	});

	test("renders document icon appropriate to file type", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		const sessionKey = await openFreshChatSession(page);

		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey,
				state: "tool_call_start",
				toolCallId: "test-csv-call",
				toolName: "send_document",
				arguments: JSON.stringify({ path: "/tmp/data.csv" }),
			},
		});

		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey,
				state: "tool_call_end",
				toolCallId: "test-csv-call",
				toolName: "send_document",
				success: true,
				result: {
					document_ref: "media/main/ghi789_data.csv",
					mime_type: "text/csv",
					filename: "data.csv",
					size_bytes: 256,
				},
			},
		});

		const csvDoc = page.locator(".document-container").filter({ hasText: "data.csv" });
		await expect(csvDoc).toBeVisible({ timeout: 10_000 });

		// Document icon should be present
		const iconEl = csvDoc.locator(".document-icon");
		await expect(iconEl).toBeVisible();
		const iconText = await iconEl.textContent();
		expect(iconText.length).toBeGreaterThan(0);

		expect(pageErrors).toEqual([]);
	});
});
