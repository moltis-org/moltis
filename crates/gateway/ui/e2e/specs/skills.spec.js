const { expect, test } = require("@playwright/test");
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

		await expect(page.getByPlaceholder("owner/repo or full URL (e.g. anthropics/skills)")).toBeVisible();
		await expect(page.getByRole("button", { name: "Install", exact: true }).first()).toBeVisible();
	});

	test("featured repos shown", async ({ page }) => {
		await navigateAndWait(page, "/skills");

		await expect(page.getByRole("heading", { name: "Featured Repositories", exact: true })).toBeVisible();
		await expect(page.getByRole("link", { name: "openclaw/skills", exact: true })).toBeVisible();
	});

	test("page has no JS errors", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/skills");
		expect(pageErrors).toEqual([]);
	});
});
