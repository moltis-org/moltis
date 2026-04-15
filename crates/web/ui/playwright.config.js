const { defineConfig } = require("@playwright/test");
const { execFileSync } = require("node:child_process");

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

function resolvePort(envVar, usedPortSet) {
	var configured = process.env[envVar];
	if (configured && configured !== "0") {
		usedPortSet.add(configured);
		return configured;
	}
	var picked = pickFreePort();
	while (usedPortSet.has(picked)) {
		picked = pickFreePort();
	}
	process.env[envVar] = picked;
	usedPortSet.add(picked);
	return picked;
}

const usedPorts = new Set();
const port = resolvePort("MOLTIS_E2E_PORT", usedPorts);
const baseURL = process.env.MOLTIS_E2E_BASE_URL || `http://127.0.0.1:${port}`;

const onboardingPort = resolvePort("MOLTIS_E2E_ONBOARDING_PORT", usedPorts);
const onboardingBaseURL = process.env.MOLTIS_E2E_ONBOARDING_BASE_URL || `http://127.0.0.1:${onboardingPort}`;

const onboardingAuthPort = resolvePort("MOLTIS_E2E_ONBOARDING_AUTH_PORT", usedPorts);
const onboardingAuthBaseURL = `http://127.0.0.1:${onboardingAuthPort}`;

const oauthPort = resolvePort("MOLTIS_E2E_OAUTH_PORT", usedPorts);
const oauthBaseURL = `http://127.0.0.1:${oauthPort}`;
const onboardingAnthropicPort = resolvePort("MOLTIS_E2E_ONBOARDING_ANTHROPIC_PORT", usedPorts);
const onboardingAnthropicBaseURL =
	process.env.MOLTIS_E2E_ONBOARDING_ANTHROPIC_BASE_URL || `http://127.0.0.1:${onboardingAnthropicPort}`;
const openaiLivePort = resolvePort("MOLTIS_E2E_OPENAI_LIVE_PORT", usedPorts);
const openaiLiveBaseURL = process.env.MOLTIS_E2E_OPENAI_LIVE_BASE_URL || `http://127.0.0.1:${openaiLivePort}`;
const openAiLiveKey = process.env.MOLTIS_E2E_OPENAI_API_KEY || process.env.OPENAI_API_KEY || "";
const enableOpenAiLiveProject = openAiLiveKey !== "";
const ollamaQwenLiveEnabled = process.env.MOLTIS_E2E_OLLAMA_QWEN_LIVE === "1";
const ollamaQwenLivePort = resolvePort("MOLTIS_E2E_OLLAMA_QWEN_LIVE_PORT", usedPorts);
const ollamaQwenLiveBaseURL =
	process.env.MOLTIS_E2E_OLLAMA_QWEN_LIVE_BASE_URL || `http://127.0.0.1:${ollamaQwenLivePort}`;
// Reliability first: fresh local gateway instances by default avoid
// hidden cross-run state leaks. Set MOLTIS_E2E_REUSE_SERVER=1 to trade
// determinism for faster startup in ad-hoc local runs.
const reuseExistingServer = !process.env.CI && process.env.MOLTIS_E2E_REUSE_SERVER === "1";
const projects = [
	{
		name: "default",
		testIgnore: [
			/auth\.spec/,
			/onboarding\.spec/,
			/onboarding-openai\.spec/,
			/onboarding-auth\.spec/,
			/onboarding-anthropic\.spec/,
			/openai-live\.spec/,
			/ollama-qwen-live\.spec/,
			/oauth\.spec/,
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
		name: "oauth",
		testMatch: /oauth\.spec/,
		use: {
			baseURL: oauthBaseURL,
		},
	},
	{
		name: "onboarding-anthropic",
		testMatch: /onboarding-anthropic\.spec/,
		use: {
			baseURL: onboardingAnthropicBaseURL,
		},
	},
];

if (enableOpenAiLiveProject) {
	projects.push({
		name: "openai-live",
		testMatch: /openai-live\.spec/,
		use: {
			baseURL: openaiLiveBaseURL,
		},
	});
}

if (ollamaQwenLiveEnabled) {
	projects.push({
		name: "ollama-qwen-live",
		testMatch: /ollama-qwen-live\.spec/,
		use: {
			baseURL: ollamaQwenLiveBaseURL,
		},
	});
}

const webServer = [
	{
		command: "./e2e/start-gateway.sh",
		cwd: __dirname,
		url: `${baseURL}/health`,
		reuseExistingServer: reuseExistingServer,
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
		reuseExistingServer: reuseExistingServer,
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
		reuseExistingServer: reuseExistingServer,
		timeout: 300_000,
		env: {
			...process.env,
			MOLTIS_E2E_ONBOARDING_AUTH_PORT: onboardingAuthPort,
		},
	},
	{
		command: "./e2e/start-gateway-oauth.sh",
		cwd: __dirname,
		url: `${oauthBaseURL}/health`,
		reuseExistingServer: reuseExistingServer,
		timeout: 300_000,
		env: {
			...process.env,
			MOLTIS_E2E_OAUTH_PORT: oauthPort,
		},
	},
	{
		command: "./e2e/start-gateway-onboarding-anthropic.sh",
		cwd: __dirname,
		url: `${onboardingAnthropicBaseURL}/health`,
		reuseExistingServer: reuseExistingServer,
		timeout: 300_000,
		env: {
			...process.env,
			MOLTIS_E2E_ONBOARDING_ANTHROPIC_PORT: onboardingAnthropicPort,
		},
	},
];

if (enableOpenAiLiveProject) {
	webServer.push({
		command: "./e2e/start-gateway-openai-live.sh",
		cwd: __dirname,
		url: `${openaiLiveBaseURL}/health`,
		reuseExistingServer: reuseExistingServer,
		timeout: 300_000,
		env: {
			...process.env,
			MOLTIS_E2E_OPENAI_LIVE_PORT: openaiLivePort,
		},
	});
}

if (ollamaQwenLiveEnabled) {
	webServer.push({
		command: "./e2e/start-gateway-ollama-qwen-live.sh",
		cwd: __dirname,
		url: `${ollamaQwenLiveBaseURL}/health`,
		reuseExistingServer: reuseExistingServer,
		timeout: 300_000,
		env: {
			...process.env,
			MOLTIS_E2E_OLLAMA_QWEN_LIVE_PORT: ollamaQwenLivePort,
		},
	});
}

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
		locale: "en-US",
		trace: "retain-on-failure",
		screenshot: "only-on-failure",
		video: "retain-on-failure",
	},
	projects,
	webServer,
});
