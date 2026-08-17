const { expect, test } = require("../base-test");
const { navigateAndWait, watchPageErrors } = require("../helpers");

test.describe("Settings > Imports > Moltis data", () => {
	test("managed Files export is opt-in and passed to the API", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		let exportUrl = "";
		await page.route("**/api/data/export?*", async (route) => {
			exportUrl = route.request().url();
			await route.fulfill({
				status: 200,
				contentType: "application/gzip",
				headers: { "content-disposition": 'attachment; filename="backup.tar.gz"' },
				body: "archive",
			});
		});
		await navigateAndWait(page, "/settings/import");

		const includeFiles = page.getByRole("checkbox", { name: "Include Files", exact: true });
		await expect(includeFiles).not.toBeChecked();
		await includeFiles.check();
		await page.getByRole("button", { name: "Download backup (.tar.gz)", exact: true }).click();

		await expect.poll(() => exportUrl).toContain("include_files=true");
		expect(pageErrors).toEqual([]);
	});

	test("changing conflict policy refreshes preview before apply", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		const previewPolicies = [];
		let appliedPolicy = "";
		await page.route("**/api/data/import/preview?*", async (route) => {
			const policy = new URL(route.request().url()).searchParams.get("conflict") || "";
			previewPolicies.push(policy);
			await route.fulfill({
				contentType: "application/json",
				body: JSON.stringify({
					manifest: {
						format_version: 1,
						moltis_version: `preview-${policy}`,
						created_at: "2026-08-16T00:00:00Z",
						inventory: {
							config_files: [],
							workspace_files: [],
							has_moltis_db: false,
							has_memory_db: false,
							session_files: [],
							media_files: [],
							managed_files: { files: ["report.pdf"], directories: [], total_bytes: 3 },
						},
					},
					imported: [],
					skipped: [
						{
							category: "files",
							path: "files/report.pdf",
							action: policy === "overwrite" ? "would overwrite" : "would skip (exists)",
						},
					],
					warnings: [],
				}),
			});
		});
		await page.route("**/api/data/import?*", async (route) => {
			appliedPolicy = new URL(route.request().url()).searchParams.get("conflict") || "";
			await route.fulfill({
				contentType: "application/json",
				body: JSON.stringify({
					manifest: {
						format_version: 1,
						moltis_version: "applied",
						created_at: "2026-08-16T00:00:00Z",
						inventory: {
							config_files: [],
							workspace_files: [],
							has_moltis_db: false,
							has_memory_db: false,
							session_files: [],
							media_files: [],
							managed_files: { files: [], directories: [], total_bytes: 0 },
						},
					},
					imported: [],
					skipped: [],
					warnings: [],
				}),
			});
		});

		await navigateAndWait(page, "/settings/import");
		await page.locator('input[type="file"][accept*=".tar.gz"]').setInputFiles({
			name: "backup.tar.gz",
			mimeType: "application/gzip",
			buffer: Buffer.from("archive"),
		});
		await expect(page.getByText("Moltis preview-skip", { exact: false })).toBeVisible();
		await page.getByRole("radio", { name: "Overwrite", exact: true }).check();
		await expect(page.getByText("Moltis preview-overwrite", { exact: false })).toBeVisible();
		await expect(page.getByText("1 will be overwritten", { exact: true })).toBeVisible();
		await page.getByRole("button", { name: "Apply import", exact: true }).click();
		await expect.poll(() => appliedPolicy).toBe("overwrite");
		expect(previewPolicies).toEqual(["skip", "overwrite"]);
		expect(pageErrors).toEqual([]);
	});
});
