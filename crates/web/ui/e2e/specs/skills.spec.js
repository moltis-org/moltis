const { expect, test } = require("../base-test");
const { navigateAndWait, watchPageErrors } = require("../helpers");

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
		await expect(page.getByPlaceholder("/path/to/skill-bundle.tar.gz")).toBeVisible();
		await expect(page.getByRole("button", { name: "Import Bundle", exact: true })).toBeVisible();
	});

	test("featured repos shown", async ({ page }) => {
		await navigateAndWait(page, "/skills");

		// Featured section is behind the Repositories tab
		await page.getByRole("tab", { name: /Repositories/ }).click();
		await expect(page.getByRole("heading", { name: "Featured Repositories", exact: true })).toBeVisible();
		await expect(page.getByRole("link", { name: "openclaw/skills", exact: true })).toBeVisible();
	});

	test("page has no JS errors", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/skills");
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
