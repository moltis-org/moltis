const { expect, test } = require("../base-test");
const { expectPageContentMounted, watchPageErrors } = require("../helpers");

test("unknown browser route renders standalone 404 with a home link", async ({ page }) => {
	const pageErrors = watchPageErrors(page);

	const response = await page.goto("/definitely-not-a-route", {
		waitUntil: "domcontentloaded",
	});

	expect(response).not.toBeNull();
	expect(response.status()).toBe(404);
	await expect(page.getByRole("heading", { name: "404", exact: true })).toBeVisible();
	await expect(page.getByText("This page could not be found.", { exact: true })).toBeVisible();

	const homeLink = page.getByRole("link", {
		name: "Go to home page",
		exact: true,
	});
	await expect(homeLink).toBeVisible();
	await homeLink.click();

	await expect(page).toHaveURL(/\/chats\/main$/);
	await expectPageContentMounted(page);
	expect(pageErrors).toEqual([]);
});
