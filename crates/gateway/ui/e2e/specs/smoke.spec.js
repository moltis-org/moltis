const { expect, test } = require("@playwright/test");
const { expectPageContentMounted, watchPageErrors } = require("../helpers");

test("app shell loads chat route instead of onboarding", async ({ page }) => {
	const pageErrors = watchPageErrors(page);

	await page.goto("/");

	await expect(page).toHaveURL(/\/chats\/main$/);
	await expectPageContentMounted(page);
	await expect(page.locator("#sessionsPanel")).toBeVisible();
	await expect(page.locator("#chatInput")).toBeVisible();
	await expect(page.locator("#statusDot")).toBeVisible();
	// statusDot should reach "connected" class; statusText is cleared to "" when connected
	await expect(page.locator("#statusDot")).toHaveClass(/connected/, { timeout: 15_000 });

	expect(pageErrors).toEqual([]);
});

const routeCases = [
	{
		path: "/crons/jobs",
		expectedUrl: /\/crons\/jobs$/,
		activeNav: "/crons",
		heading: "Cron Jobs",
	},
	{
		path: "/monitoring",
		expectedUrl: /\/monitoring$/,
		activeNav: "/monitoring",
		heading: "Monitoring",
	},
	{
		path: "/skills",
		expectedUrl: /\/skills$/,
		activeNav: "/skills",
		heading: "Skills",
	},
	{
		path: "/projects",
		expectedUrl: /\/projects$/,
		activeNav: null,
		heading: "Repositories",
	},
	{
		path: "/settings",
		expectedUrl: /\/settings\/identity$/,
		activeNav: "/settings",
		heading: "Identity",
	},
];

for (const routeCase of routeCases) {
	test(`route ${routeCase.path} renders without uncaught errors`, async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await page.goto(routeCase.path);

		await expect(page).toHaveURL(routeCase.expectedUrl);
		await expectPageContentMounted(page);
		if (routeCase.activeNav) {
			await expect(page.locator(`a.nav-link[href="${routeCase.activeNav}"]`)).toHaveClass(/active/);
		}
		await expect(
			page.getByRole("heading", {
				name: routeCase.heading,
				exact: true,
			}),
		).toBeVisible();

		expect(pageErrors).toEqual([]);
	});
}
