const { expect, test } = require("../base-test");
const { navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

test.describe("send_document rendering", () => {
	test("renders document card with filename and download link for document_ref", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/chats/main");
		await waitForWsConnected(page);

		// Inject a fake tool-result event containing document_ref
		await page.evaluate(async () => {
			const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			if (!appScript) throw new Error("app module script not found");
			const appUrl = new URL(appScript.src, window.location.origin);
			const prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			const events = await import(`${prefix}js/events.js`);

			// Simulate tool_call event to create the tool card
			events.eventListeners.forEach((fn) => {
				fn({
					type: "tool_call",
					session: "main",
					data: {
						id: "test-doc-call",
						name: "send_document",
						args: { path: "/tmp/report.pdf" },
					},
				});
			});

			// Simulate tool_result event with document_ref
			events.eventListeners.forEach((fn) => {
				fn({
					type: "tool_result",
					session: "main",
					data: {
						id: "test-doc-call",
						success: true,
						result: {
							document_ref: "media/main/abc123_report.pdf",
							mime_type: "application/pdf",
							filename: "report.pdf",
							size_bytes: 12345,
							sent: true,
						},
					},
				});
			});
		});

		// Verify the document card renders
		const docContainer = page.locator(".document-container").first();
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
		expect(href).toContain("/api/sessions/main/media/abc123_report.pdf");

		// PDF should open in new tab (not trigger download)
		const target = await downloadBtn.getAttribute("target");
		expect(target).toBe("_blank");

		expect(pageErrors).toEqual([]);
	});

	test("renders document card for zip file with download attribute", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/chats/main");
		await waitForWsConnected(page);

		await page.evaluate(async () => {
			const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			if (!appScript) throw new Error("app module script not found");
			const appUrl = new URL(appScript.src, window.location.origin);
			const prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			const events = await import(`${prefix}js/events.js`);

			events.eventListeners.forEach((fn) => {
				fn({
					type: "tool_call",
					session: "main",
					data: {
						id: "test-zip-call",
						name: "send_document",
						args: { path: "/tmp/archive.zip" },
					},
				});
			});

			events.eventListeners.forEach((fn) => {
				fn({
					type: "tool_result",
					session: "main",
					data: {
						id: "test-zip-call",
						success: true,
						result: {
							document_ref: "media/main/def456_archive.zip",
							mime_type: "application/zip",
							filename: "archive.zip",
							size_bytes: 5242880,
							sent: true,
						},
					},
				});
			});
		});

		const docContainer = page.locator(".document-container").first();
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
		await navigateAndWait(page, "/chats/main");
		await waitForWsConnected(page);

		await page.evaluate(async () => {
			const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			if (!appScript) throw new Error("app module script not found");
			const appUrl = new URL(appScript.src, window.location.origin);
			const prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			const events = await import(`${prefix}js/events.js`);

			events.eventListeners.forEach((fn) => {
				fn({
					type: "tool_call",
					session: "main",
					data: {
						id: "test-csv-call",
						name: "send_document",
						args: { path: "/tmp/data.csv" },
					},
				});
			});

			events.eventListeners.forEach((fn) => {
				fn({
					type: "tool_result",
					session: "main",
					data: {
						id: "test-csv-call",
						success: true,
						result: {
							document_ref: "media/main/ghi789_data.csv",
							mime_type: "text/csv",
							filename: "data.csv",
							size_bytes: 256,
							sent: true,
						},
					},
				});
			});
		});

		const docContainer = page.locator(".document-container").first();
		await expect(docContainer).toBeVisible({ timeout: 5_000 });

		// Document icon should be present
		const iconEl = docContainer.locator(".document-icon");
		await expect(iconEl).toBeVisible();
		const iconText = await iconEl.textContent();
		expect(iconText.length).toBeGreaterThan(0);

		expect(pageErrors).toEqual([]);
	});
});
