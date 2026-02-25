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

test("index page exposes OG and Twitter share metadata", async ({ page }) => {
	const pageErrors = watchPageErrors(page);

	await page.goto("/");
	await expect(page).toHaveURL(/\/chats\/main$/);

	await expect.poll(() => page.locator('meta[property="og:title"]').getAttribute("content")).toContain("AI assistant");
	await expect
		.poll(() => page.locator('meta[property="og:description"]').getAttribute("content"))
		.toContain("personal AI assistant");
	await expect(page.locator('meta[property="og:image"]')).toHaveAttribute(
		"content",
		"https://www.moltis.org/og-social.jpg?v=4",
	);
	await expect(page.locator('meta[name="twitter:card"]')).toHaveAttribute("content", "summary_large_image");
	await expect(page.locator('meta[name="twitter:image"]')).toHaveAttribute(
		"content",
		"https://www.moltis.org/og-social.jpg?v=4",
	);

	expect(pageErrors).toEqual([]);
});

test("mobile menu drives settings and sessions", async ({ page }) => {
	const pageErrors = watchPageErrors(page);
	await page.setViewportSize({ width: 390, height: 844 });

	await page.goto("/");
	await expect(page).toHaveURL(/\/chats\/main$/);
	await expectPageContentMounted(page);

	await expect(page.locator("#settingsBtn")).toBeHidden();
	await expect(page.locator("#mobileMenuBtn")).toBeVisible();
	await page.locator("#mobileMenuBtn").click();
	await expect(page.locator("#mobileMenuPanel")).toHaveClass(/open/);
	await page.locator("#mobileMenuSettingsBtn").click();
	await expect(page).toHaveURL(/\/settings\/identity$/);
	await expect(page.locator(".settings-sidebar")).toHaveCount(0);
	await page.locator(".settings-mobile-menu-btn").click();
	await expect(page.locator(".settings-sidebar")).toBeVisible();
	await page.locator(".settings-nav-item", { hasText: "Memory" }).click();
	await expect(page).toHaveURL(/\/settings\/memory$/);
	await expect(page.locator(".settings-sidebar")).toHaveCount(0);
	await page.locator(".settings-mobile-menu-btn").click();
	var voiceNav = page.locator(".settings-nav-item", { hasText: "Voice" });
	await voiceNav.scrollIntoViewIfNeeded();
	await voiceNav.click();
	await expect(page).toHaveURL(/\/settings\/voice$/);
	await expect(page.locator(".settings-sidebar")).toHaveCount(0);
	await page.locator(".settings-mobile-menu-btn").click();
	var heartbeatNav = page.locator(".settings-nav-item", { hasText: "Heartbeat" });
	await heartbeatNav.scrollIntoViewIfNeeded();
	await heartbeatNav.click();
	await expect(page).toHaveURL(/\/settings\/heartbeat$/);
	await expect(
		page.getByRole("heading", {
			name: "Heartbeat",
			exact: true,
		}),
	).toBeVisible();

	await page.goto("/chats/main");
	await expectPageContentMounted(page);
	await expect(page.locator("#sessionsToggle")).toBeHidden();
	await page.locator("#mobileMenuBtn").click();
	await page.locator("#mobileMenuSessionsBtn").click();
	await expect(page.locator("#sessionsPanel")).toHaveClass(/open/);
	await expect(page.locator("#sessionsOverlay")).toHaveClass(/visible/);

	expect(pageErrors).toEqual([]);
});

const routeCases = [
	{
		path: "/settings/crons",
		expectedUrl: /\/settings\/crons$/,
		heading: "Cron Jobs",
	},
	{
		path: "/monitoring",
		expectedUrl: /\/monitoring$/,
		heading: "Monitoring",
	},
	{
		path: "/skills",
		expectedUrl: /\/skills$/,
		heading: "Skills",
	},
	{
		path: "/projects",
		expectedUrl: /\/projects$/,
		heading: "Repositories",
	},
	{
		path: "/settings",
		expectedUrl: /\/settings\/identity$/,
		settingsActive: true,
		heading: "Identity",
	},
];

for (const routeCase of routeCases) {
	test(`route ${routeCase.path} renders without uncaught errors`, async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await page.goto(routeCase.path);

		await expect(page).toHaveURL(routeCase.expectedUrl);
		await expectPageContentMounted(page);
		if (routeCase.settingsActive) {
			await expect(page.locator("#settingsBtn")).toHaveClass(/active/);
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
