const { expect, test } = require("../base-test");
const { navigateAndWait, watchPageErrors } = require("../helpers");

function entry(name, kind, options = {}) {
	return {
		name,
		path: options.path || name,
		kind,
		sizeBytes: options.sizeBytes ?? (kind === "file" ? 1 : null),
		modifiedAt: options.modifiedAt || null,
	};
}

function filePathHeader(request) {
	const value = request.headers()["x-moltis-file-path"] || "base64url:";
	if (!value.startsWith("base64url:")) throw new Error(`Unexpected Files path header: ${value}`);
	return Buffer.from(value.slice("base64url:".length), "base64url").toString("utf8");
}

async function mockFileRoutes(page, handler) {
	await page.route("**/api/files/**", async (route) => {
		const handled = await handler(route, route.request());
		if (!handled) {
			await route.fulfill({
				status: 500,
				contentType: "application/json",
				body: JSON.stringify({ message: "Unexpected files API request" }),
			});
		}
	});
}

async function fulfillEntries(route, path, entries) {
	await route.fulfill({ contentType: "application/json", body: JSON.stringify({ path, entries }) });
}

async function openFiles(page) {
	await navigateAndWait(page, "/settings/files");
	await expect(page.getByRole("heading", { name: "Files", exact: true })).toBeVisible();
}

async function firstCellNames(page) {
	return page.getByRole("region", { name: "Files table" }).locator("tbody tr td:first-child").allTextContents();
}

test.describe("Settings > Files", () => {
	test("sorts folders first and navigates with breadcrumbs", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		const rootEntries = [
			entry("zeta-folder", "directory", { modifiedAt: "2025-02-01T10:00:00Z" }),
			entry("alpha-folder", "directory", { modifiedAt: "2022-02-01T10:00:00Z" }),
			entry("zeta.txt", "file", { sizeBytes: 100, modifiedAt: "2024-02-01T10:00:00Z" }),
			entry("alpha.txt", "file", { sizeBytes: 10, modifiedAt: "2021-02-01T10:00:00Z" }),
			entry("beta.txt", "file", { sizeBytes: 1000, modifiedAt: "2023-02-01T10:00:00Z" }),
		];
		await mockFileRoutes(page, async (route, request) => {
			if (request.method() !== "GET" || new URL(request.url()).pathname !== "/api/files/entries") return false;
			const path = filePathHeader(request);
			if (path === "alpha-folder") {
				await fulfillEntries(route, path, [entry("inside.txt", "file", { path: "alpha-folder/inside.txt" })]);
			} else {
				await fulfillEntries(route, "", rootEntries);
			}
			return true;
		});

		await openFiles(page);
		await expect
			.poll(() => firstCellNames(page))
			.toEqual(["alpha-folder", "zeta-folder", "alpha.txt", "beta.txt", "zeta.txt"]);
		await expect(page.getByRole("columnheader", { name: /Filename/ })).toHaveAttribute("aria-sort", "ascending");

		await page.getByRole("button", { name: /Date Modified/ }).click();
		await expect(page.getByRole("columnheader", { name: /Date Modified/ })).toHaveAttribute("aria-sort", "descending");
		await expect
			.poll(() => firstCellNames(page))
			.toEqual(["zeta-folder", "alpha-folder", "zeta.txt", "beta.txt", "alpha.txt"]);

		await page.getByRole("button", { name: /Size/ }).click();
		await expect(page.getByRole("columnheader", { name: /Size/ })).toHaveAttribute("aria-sort", "descending");
		await expect
			.poll(() => firstCellNames(page))
			.toEqual(["alpha-folder", "zeta-folder", "beta.txt", "zeta.txt", "alpha.txt"]);

		await page.getByRole("button", { name: "alpha-folder", exact: true }).click();
		await expect(
			page.getByRole("navigation", { name: "File path" }).getByText("alpha-folder", { exact: true }),
		).toBeVisible();
		await expect(page.getByText("inside.txt", { exact: true })).toBeVisible();
		await page
			.getByRole("navigation", { name: "File path" })
			.getByRole("button", { name: "Files", exact: true })
			.click();
		await expect(page.getByText("beta.txt", { exact: true })).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("uploads files, folder selections, and dropped directory trees with relative paths", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		const uploadedPaths = [];
		const createdPaths = [];
		await mockFileRoutes(page, async (route, request) => {
			const pathname = new URL(request.url()).pathname;
			if (request.method() === "GET" && pathname === "/api/files/entries") {
				await fulfillEntries(route, "", []);
				return true;
			}
			if (request.method() === "PUT" && pathname === "/api/files/content") {
				uploadedPaths.push(filePathHeader(request));
				await route.fulfill({ status: 204, body: "" });
				return true;
			}
			if (request.method() === "POST" && pathname === "/api/files/directories") {
				createdPaths.push(request.postDataJSON().path);
				await route.fulfill({ status: 204, body: "" });
				return true;
			}
			return false;
		});

		await openFiles(page);
		await page.getByLabel("Choose files to upload").setInputFiles({
			name: "plain.txt",
			mimeType: "text/plain",
			buffer: Buffer.from("plain"),
		});
		await expect.poll(() => uploadedPaths).toContain("plain.txt");
		await page.getByLabel("Choose files to upload").setInputFiles({
			name: "健康-résumé.pdf",
			mimeType: "application/pdf",
			buffer: Buffer.from("pdf"),
		});
		await expect.poll(() => uploadedPaths).toContain("健康-résumé.pdf");

		await page.evaluate(() => {
			const input = document.querySelector('input[aria-label="Choose folder to upload"]');
			if (!(input instanceof HTMLInputElement)) throw new Error("Folder input not found");
			const file = new File(["nested"], "nested.txt", { type: "text/plain" });
			Object.defineProperty(file, "webkitRelativePath", { value: "selected/deep/nested.txt" });
			const transfer = new DataTransfer();
			transfer.items.add(file);
			Object.defineProperty(input, "files", { configurable: true, value: transfer.files });
			input.dispatchEvent(new Event("change", { bubbles: true }));
		});
		await expect.poll(() => uploadedPaths).toContain("selected/deep/nested.txt");
		await expect.poll(() => createdPaths).toEqual(expect.arrayContaining(["selected", "selected/deep"]));

		await page.getByRole("region", { name: "Files table" }).evaluate((element) => {
			function fileEntry(name, body) {
				return { isFile: true, isDirectory: false, name, file: (done) => done(new File([body], name)) };
			}
			function directoryEntry(name, batches) {
				return {
					isFile: false,
					isDirectory: true,
					name,
					createReader() {
						let index = 0;
						return { readEntries: (done) => done(batches[index++] || []) };
					},
				};
			}
			const empty = directoryEntry("empty", [[]]);
			const nested = directoryEntry("nested", [[fileEntry("deep.txt", "deep"), empty], []]);
			const root = directoryEntry("dropped", [[fileEntry("root.txt", "root")], [nested], []]);
			const event = new Event("drop", { bubbles: true, cancelable: true });
			Object.defineProperty(event, "dataTransfer", {
				value: { items: [{ webkitGetAsEntry: () => root }], files: [] },
			});
			element.dispatchEvent(event);
		});
		await expect
			.poll(() => uploadedPaths)
			.toEqual(expect.arrayContaining(["dropped/root.txt", "dropped/nested/deep.txt"]));
		await expect
			.poll(() => createdPaths)
			.toEqual(expect.arrayContaining(["dropped", "dropped/nested", "dropped/nested/empty"]));
		expect(pageErrors).toEqual([]);
	});

	test("requires explicit confirmation before retrying an upload conflict", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		const uploadHeaders = [];
		await mockFileRoutes(page, async (route, request) => {
			const pathname = new URL(request.url()).pathname;
			if (request.method() === "GET" && pathname === "/api/files/entries") {
				await fulfillEntries(route, "", []);
				return true;
			}
			if (request.method() === "PUT" && pathname === "/api/files/content") {
				uploadHeaders.push(request.headers());
				if (uploadHeaders.length === 1) {
					await route.fulfill({
						status: 409,
						contentType: "application/json",
						body: JSON.stringify({ message: "File exists" }),
					});
				} else {
					await route.fulfill({ status: 204, body: "" });
				}
				return true;
			}
			return false;
		});

		await openFiles(page);
		await page.getByLabel("Choose files to upload").setInputFiles({
			name: "conflict.txt",
			mimeType: "text/plain",
			buffer: Buffer.from("new"),
		});
		const dialog = page.getByRole("alertdialog", { name: "Replace file?" });
		await expect(dialog).toBeVisible();
		expect(uploadHeaders).toHaveLength(1);
		await dialog.getByRole("button", { name: "Overwrite", exact: true }).click();
		await expect.poll(() => uploadHeaders.length).toBe(2);
		expect(uploadHeaders[1]["x-moltis-overwrite"]).toBe("true");
		await expect(page.getByRole("status")).toContainText("1 file uploaded");
		expect(pageErrors).toEqual([]);
	});

	test("creates, renames, moves, recursively deletes, and displays download errors", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		const directoryRequests = [];
		const moveRequests = [];
		const deleteRequests = [];
		await mockFileRoutes(page, async (route, request) => {
			const pathname = new URL(request.url()).pathname;
			if (request.method() === "GET" && pathname === "/api/files/entries") {
				await fulfillEntries(route, "", [entry("docs", "directory"), entry("report.txt", "file", { sizeBytes: 25 })]);
				return true;
			}
			if (request.method() === "GET" && pathname === "/api/files/content") {
				await route.fulfill({
					status: 500,
					contentType: "application/json",
					body: JSON.stringify({ message: "Download unavailable" }),
				});
				return true;
			}
			if (request.method() === "POST" && pathname === "/api/files/directories") {
				directoryRequests.push(request.postDataJSON());
				await route.fulfill({ status: 204, body: "" });
				return true;
			}
			if (request.method() === "POST" && pathname === "/api/files/move") {
				const body = request.postDataJSON();
				moveRequests.push(body);
				if (body.destination === "archive/report.txt" && !body.overwrite) {
					await route.fulfill({
						status: 409,
						contentType: "application/json",
						body: JSON.stringify({ message: "Destination exists" }),
					});
				} else {
					await route.fulfill({ status: 204, body: "" });
				}
				return true;
			}
			if (request.method() === "DELETE" && pathname === "/api/files/entries") {
				const body = request.postDataJSON();
				deleteRequests.push(body);
				if (body.recursive) {
					await route.fulfill({ status: 204, body: "" });
				} else {
					await route.fulfill({
						status: 409,
						contentType: "application/json",
						body: JSON.stringify({ message: "Folder is not empty" }),
					});
				}
				return true;
			}
			return false;
		});

		await openFiles(page);
		await page.getByRole("button", { name: "New Folder", exact: true }).click();
		let dialog = page.getByRole("dialog", { name: "New Folder" });
		await dialog.getByLabel("Folder name").fill("archive");
		await dialog.getByRole("button", { name: "Create", exact: true }).click();
		await expect.poll(() => directoryRequests).toContainEqual({ path: "archive" });

		let row = page.getByRole("row").filter({ has: page.getByText("report.txt", { exact: true }) });
		await row.getByRole("button", { name: "Rename report.txt", exact: true }).click();
		dialog = page.getByRole("dialog", { name: "Rename report.txt" });
		await dialog.getByLabel("New name").fill("summary.txt");
		await dialog.getByRole("button", { name: "Rename", exact: true }).click();
		await expect
			.poll(() => moveRequests)
			.toContainEqual({ source: "report.txt", destination: "summary.txt", overwrite: false });

		row = page.getByRole("row").filter({ has: page.getByText("report.txt", { exact: true }) });
		await row.getByRole("button", { name: "Move report.txt", exact: true }).click();
		dialog = page.getByRole("dialog", { name: "Move report.txt" });
		await dialog.getByLabel("Destination folder", { exact: true }).fill("archive");
		await dialog.getByRole("button", { name: "Move", exact: true }).click();
		const replaceDialog = page.getByRole("alertdialog", { name: "Replace existing item?" });
		await expect(replaceDialog).toBeVisible();
		await replaceDialog.getByRole("button", { name: "Replace", exact: true }).click();
		await expect
			.poll(() => moveRequests)
			.toContainEqual({
				source: "report.txt",
				destination: "archive/report.txt",
				overwrite: true,
			});

		const docsRow = page.getByRole("row").filter({ has: page.getByText("docs", { exact: true }) });
		await docsRow.getByRole("button", { name: "Delete docs", exact: true }).click();
		await page
			.getByRole("alertdialog", { name: "Delete docs?" })
			.getByRole("button", { name: "Delete", exact: true })
			.click();
		const recursiveDialog = page.getByRole("alertdialog", { name: "Delete non-empty folder?" });
		await expect(recursiveDialog).toBeVisible();
		await recursiveDialog.getByRole("button", { name: "Delete all", exact: true }).click();
		await expect
			.poll(() => deleteRequests)
			.toEqual([
				{ path: "docs", recursive: false },
				{ path: "docs", recursive: true },
			]);

		await page.evaluate(() => Object.defineProperty(window, "showSaveFilePicker", { value: undefined }));
		row = page.getByRole("row").filter({ has: page.getByText("report.txt", { exact: true }) });
		await row.getByRole("button", { name: "Download report.txt", exact: true }).click();
		await expect(page.getByRole("alert")).toContainText("Download unavailable");
		expect(pageErrors).toEqual([]);
	});

	test("refuses an oversized buffered download when streaming is unavailable", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await mockFileRoutes(page, async (route, request) => {
			const pathname = new URL(request.url()).pathname;
			if (request.method() === "GET" && pathname === "/api/files/entries") {
				await fulfillEntries(route, "", [entry("growing.bin", "file", { sizeBytes: 1 })]);
				return true;
			}
			return false;
		});

		await openFiles(page);
		await page.evaluate(() => {
			Object.defineProperty(window, "showSaveFilePicker", { value: undefined });
			const originalFetch = window.fetch.bind(window);
			window.fetch = (input, init) => {
				if (String(input).includes("/api/files/content")) {
					return Promise.resolve(
						new Response("", { status: 200, headers: { "Content-Length": String(65 * 1024 * 1024) } }),
					);
				}
				return originalFetch(input, init);
			};
		});
		const row = page.getByRole("row").filter({ has: page.getByText("growing.bin", { exact: true }) });
		await row.getByRole("button", { name: "Download growing.bin", exact: true }).click();
		await expect(page.getByRole("alert")).toContainText("cannot safely buffer this file");
		expect(pageErrors).toEqual([]);
	});

	test("shows the empty state and local path semantics", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await mockFileRoutes(page, async (route, request) => {
			if (request.method() !== "GET") return false;
			await fulfillEntries(route, "", []);
			return true;
		});
		await openFiles(page);
		await expect(
			page.getByText("This folder is empty. Upload files or drop them here.", { exact: true }),
		).toBeVisible();
		await expect(page.getByText("MOLTIS_FILES_DIR", { exact: true })).toBeVisible();
		await expect(page.getByText("/home/sandbox/files", { exact: true })).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("keeps the files table horizontally scrollable on mobile", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.setViewportSize({ width: 390, height: 800 });
		await mockFileRoutes(page, async (route, request) => {
			if (request.method() !== "GET") return false;
			await fulfillEntries(route, "", [
				entry("a-very-long-filename-for-mobile-layout.txt", "file", { sizeBytes: 9000 }),
			]);
			return true;
		});
		await openFiles(page);
		const region = page.getByRole("region", { name: "Files table" });
		await expect(region).toBeVisible();
		await expect.poll(() => region.evaluate((element) => element.scrollWidth > element.clientWidth)).toBe(true);
		expect(pageErrors).toEqual([]);
	});
});
