const { defineConfig } = require("@playwright/test");

const port = process.env.MOLTIS_E2E_PORT || "18789";
const baseURL = process.env.MOLTIS_E2E_BASE_URL || `http://127.0.0.1:${port}`;

const onboardingPort = process.env.MOLTIS_E2E_ONBOARDING_PORT || "18790";
const onboardingBaseURL = process.env.MOLTIS_E2E_ONBOARDING_BASE_URL || `http://127.0.0.1:${onboardingPort}`;

module.exports = defineConfig({
	testDir: "./e2e/specs",
	timeout: 45_000,
	expect: {
		timeout: 10_000,
	},
	fullyParallel: false,
	forbidOnly: !!process.env.CI,
	retries: process.env.CI ? 1 : 0,
	workers: 1,
	reporter: process.env.CI ? [["github"], ["html", { open: "never" }]] : [["list"], ["html", { open: "never" }]],
	use: {
		baseURL: baseURL,
		trace: "retain-on-failure",
		screenshot: "only-on-failure",
		video: "retain-on-failure",
	},
	projects: [
		{
			name: "default",
			testIgnore: [/auth\.spec/, /onboarding\.spec/],
		},
		{
			name: "auth",
			testMatch: /auth\.spec/,
			dependencies: ["default"],
		},
		{
			name: "onboarding",
			testMatch: /onboarding\.spec/,
			use: {
				baseURL: onboardingBaseURL,
			},
		},
	],
	webServer: [
		{
			command: "./e2e/start-gateway.sh",
			cwd: __dirname,
			url: `${baseURL}/health`,
			reuseExistingServer: !process.env.CI,
			timeout: 300_000,
			env: {
				...process.env,
				MOLTIS_E2E_PORT: port,
			},
		},
		{
			command: "./e2e/start-gateway-onboarding.sh",
			cwd: __dirname,
			url: `${onboardingBaseURL}/health`,
			reuseExistingServer: !process.env.CI,
			timeout: 300_000,
			env: {
				...process.env,
				MOLTIS_E2E_ONBOARDING_PORT: onboardingPort,
			},
		},
	],
});
