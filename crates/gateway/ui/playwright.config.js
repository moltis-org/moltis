const { defineConfig } = require("@playwright/test");
const { execFileSync } = require("child_process");

function pickFreePort() {
	return execFileSync(
		process.execPath,
		[
			"-e",
			"const net=require('net');const s=net.createServer();s.listen(0,'127.0.0.1',()=>{process.stdout.write(String(s.address().port));s.close();});",
		],
		{ encoding: "utf8" },
	).trim();
}

function resolvePort(envVar, usedPorts) {
	var configured = process.env[envVar];
	if (configured && configured !== "0") {
		usedPorts.add(configured);
		return configured;
	}
	var picked = pickFreePort();
	while (usedPorts.has(picked)) {
		picked = pickFreePort();
	}
	process.env[envVar] = picked;
	usedPorts.add(picked);
	return picked;
}

const usedPorts = new Set();
const port = resolvePort("MOLTIS_E2E_PORT", usedPorts);
const baseURL = process.env.MOLTIS_E2E_BASE_URL || `http://127.0.0.1:${port}`;

const onboardingPort = resolvePort("MOLTIS_E2E_ONBOARDING_PORT", usedPorts);
const onboardingBaseURL = process.env.MOLTIS_E2E_ONBOARDING_BASE_URL || `http://127.0.0.1:${onboardingPort}`;

const onboardingAuthPort = resolvePort("MOLTIS_E2E_ONBOARDING_AUTH_PORT", usedPorts);
const onboardingAuthBaseURL = `http://127.0.0.1:${onboardingAuthPort}`;

const onboardingAnthropicPort = resolvePort("MOLTIS_E2E_ONBOARDING_ANTHROPIC_PORT", usedPorts);
const onboardingAnthropicBaseURL =
	process.env.MOLTIS_E2E_ONBOARDING_ANTHROPIC_BASE_URL || `http://127.0.0.1:${onboardingAnthropicPort}`;

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
			testIgnore: [
				/auth\.spec/,
				/onboarding\.spec/,
				/onboarding-openai\.spec/,
				/onboarding-auth\.spec/,
				/onboarding-anthropic\.spec/,
			],
		},
		{
			name: "auth",
			testMatch: /\/auth\.spec/,
			dependencies: ["default"],
		},
		{
			name: "onboarding",
			testMatch: /onboarding(?:-openai)?\.spec/,
			use: {
				baseURL: onboardingBaseURL,
			},
		},
		{
			name: "onboarding-auth",
			testMatch: /onboarding-auth\.spec/,
			use: {
				baseURL: onboardingAuthBaseURL,
			},
		},
		{
			name: "onboarding-anthropic",
			testMatch: /onboarding-anthropic\.spec/,
			use: {
				baseURL: onboardingAnthropicBaseURL,
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
		{
			command: "./e2e/start-gateway-onboarding-auth.sh",
			cwd: __dirname,
			url: `${onboardingAuthBaseURL}/health`,
			reuseExistingServer: !process.env.CI,
			timeout: 300_000,
			env: {
				...process.env,
				MOLTIS_E2E_ONBOARDING_AUTH_PORT: onboardingAuthPort,
			},
		},
		{
			command: "./e2e/start-gateway-onboarding-anthropic.sh",
			cwd: __dirname,
			url: `${onboardingAnthropicBaseURL}/health`,
			reuseExistingServer: !process.env.CI,
			timeout: 300_000,
			env: {
				...process.env,
				MOLTIS_E2E_ONBOARDING_ANTHROPIC_PORT: onboardingAnthropicPort,
			},
		},
	],
});
