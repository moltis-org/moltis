const { defineConfig } = require("@playwright/test");
const { execFileSync } = require("node:child_process");
const { readFileSync, readdirSync } = require("node:fs");
const path = require("node:path");

const repoRoot = path.resolve(__dirname, "../../..");
const isCi = Boolean(process.env.CI);
const configuredShardCount = Number.parseInt(process.env.MOLTIS_E2E_SHARDS || "", 10);
const defaultShardCount = Number.isFinite(configuredShardCount) && configuredShardCount > 0 ? configuredShardCount : 4;
const processShardIndex = Number.parseInt(process.env.MOLTIS_E2E_PROCESS_SHARD_INDEX || "", 10);
const processShardTotal = Number.parseInt(process.env.MOLTIS_E2E_PROCESS_SHARD_TOTAL || "", 10);
const processDefaultShard =
	isCi &&
	Number.isFinite(processShardIndex) &&
	Number.isFinite(processShardTotal) &&
	processShardIndex > 0 &&
	processShardTotal > 0 &&
	processShardIndex <= processShardTotal;
const skipDefaultProjects = isCi && process.env.MOLTIS_E2E_SKIP_DEFAULT_PROJECTS === "1";
const onlyProject = process.env.MOLTIS_E2E_ONLY_PROJECT || "";

function includeProject(name) {
	return !onlyProject || onlyProject === name;
}

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

function escapeRegExp(value) {
	return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function defaultSpecFiles() {
	const ignored = [
		/agents\.spec\.js$/,
		/auth\.spec\.js$/,
		/onboarding\.spec\.js$/,
		/onboarding-openai\.spec\.js$/,
		/onboarding-auth\.spec\.js$/,
		/onboarding-anthropic\.spec\.js$/,
		/openai-live\.spec\.js$/,
		/ollama-qwen-live\.spec\.js$/,
		/oauth\.spec\.js$/,
	];
	return readdirSync(path.join(__dirname, "e2e/specs"))
		.filter((file) => file.endsWith(".spec.js") && !ignored.some((pattern) => pattern.test(file)))
		.sort();
}

const defaultSpecWeights = new Map([
	// Keep CI shards balanced by observed runtime. Some settings specs do much
	// more UI setup than their raw test count suggests.
	["settings-nav.spec.js", 34],
	["settings-channels.spec.js", 32],
	["sessions.spec.js", 26],
	["cron.spec.js", 21],
	["chat-input.spec.js", 20],
	["websocket.spec.js", 19],
	["command-palette.spec.js", 17],
	["mcp.spec.js", 12],
	["chat-autoscroll.spec.js", 11],
	["sandboxes.spec.js", 11],
	["channels-matrix.spec.js", 10],
	["skills.spec.js", 10],
	["i18n.spec.js", 9],
	["providers.spec.js", 9],
	["smoke.spec.js", 9],
]);

function countSpecTests(file) {
	const source = readFileSync(path.join(__dirname, "e2e/specs", file), "utf8");
	return source.match(/\btest(?:\.(?:fixme|only|skip))?\s*\(/g)?.length || 1;
}

function specWeight(file) {
	return defaultSpecWeights.get(file) || countSpecTests(file);
}

function shardSpecFiles(files, shardCount) {
	const shards = Array.from({ length: shardCount }, () => ({ files: [], weight: 0 }));
	files
		.toSorted((a, b) => specWeight(b) - specWeight(a) || a.localeCompare(b))
		.forEach((file) => {
			const shard = shards.toSorted((a, b) => a.weight - b.weight)[0];
			shard.files.push(file);
			shard.weight += specWeight(file);
		});
	return shards
		.filter((shard) => shard.files.length > 0)
		.map((shard) => {
			shard.files.sort();
			return shard.files;
		});
}

function matchSpecFiles(files) {
	return files.map((file) => new RegExp(`${escapeRegExp(file)}$`));
}

const usedPorts = new Set();
const defaultPorts = skipDefaultProjects ? [] : [resolvePort("MOLTIS_E2E_PORT", usedPorts)];
const defaultBaseURLs = defaultPorts.map((defaultPort) => process.env.MOLTIS_E2E_BASE_URL || `http://127.0.0.1:${defaultPort}`);
const port = defaultPorts[0];
const baseURL = defaultBaseURLs[0];

const agentsPort = resolvePort("MOLTIS_E2E_AGENTS_PORT", usedPorts);
const agentsBaseURL = process.env.MOLTIS_E2E_AGENTS_BASE_URL || `http://127.0.0.1:${agentsPort}`;

const authPort = resolvePort("MOLTIS_E2E_AUTH_PORT", usedPorts);
const authBaseURL = process.env.MOLTIS_E2E_AUTH_BASE_URL || `http://127.0.0.1:${authPort}`;

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
const defaultProjectIgnore = [
	/agents\.spec/,
	/auth\.spec/,
	/onboarding\.spec/,
	/onboarding-openai\.spec/,
	/onboarding-auth\.spec/,
	/onboarding-anthropic\.spec/,
	/openai-live\.spec/,
	/ollama-qwen-live\.spec/,
	/oauth\.spec/,
];
const defaultProjects = (() => {
	if (skipDefaultProjects || !includeProject("default")) return [];
	if (processDefaultShard) {
		const shardFiles = shardSpecFiles(defaultSpecFiles(), processShardTotal)[processShardIndex - 1] || [];
		return [
			{
				name: "default",
				testMatch: matchSpecFiles(shardFiles),
				use: {
					baseURL,
				},
			},
		];
	}
	return [
		{
			name: "default",
			testIgnore: defaultProjectIgnore,
			use: {
				baseURL,
			},
		},
	];
})();
const projects = defaultProjects.concat(
	[
		includeProject("agents")
			? isCi
				? {
						name: "agents",
						testMatch: /agents\.spec/,
						use: {
							baseURL: agentsBaseURL,
						},
					}
				: {
						name: "agents",
						testMatch: /agents\.spec/,
						dependencies: ["default"],
					}
			: null,
		includeProject("auth")
			? isCi
				? {
						name: "auth",
						testMatch: /\/auth\.spec/,
						use: {
							baseURL: authBaseURL,
						},
					}
				: {
						name: "auth",
						testMatch: /\/auth\.spec/,
						dependencies: ["default"],
					}
			: null,
		includeProject("onboarding")
			? {
					name: "onboarding",
					testMatch: /onboarding(?:-openai)?\.spec/,
					use: {
						baseURL: onboardingBaseURL,
					},
				}
			: null,
		includeProject("onboarding-auth")
			? {
					name: "onboarding-auth",
					testMatch: /onboarding-auth\.spec/,
					use: {
						baseURL: onboardingAuthBaseURL,
					},
				}
			: null,
		includeProject("oauth")
			? {
					name: "oauth",
					testMatch: /oauth\.spec/,
					use: {
						baseURL: oauthBaseURL,
					},
				}
			: null,
		includeProject("onboarding-anthropic")
			? {
					name: "onboarding-anthropic",
					testMatch: /onboarding-anthropic\.spec/,
					use: {
						baseURL: onboardingAnthropicBaseURL,
					},
				}
			: null,
	].filter(Boolean),
);

if (enableOpenAiLiveProject && includeProject("openai-live")) {
	projects.push({
		name: "openai-live",
		testMatch: /openai-live\.spec/,
		use: {
			baseURL: openaiLiveBaseURL,
		},
	});
}

if (ollamaQwenLiveEnabled && includeProject("ollama-qwen-live")) {
	projects.push({
		name: "ollama-qwen-live",
		testMatch: /ollama-qwen-live\.spec/,
		use: {
			baseURL: ollamaQwenLiveBaseURL,
		},
	});
}

function gatewayServer({ baseURL: serverBaseURL, name, port: serverPort }) {
	return {
		command: "./e2e/start-gateway.sh",
		cwd: __dirname,
		url: `${serverBaseURL}/health`,
		reuseExistingServer: reuseExistingServer,
		timeout: 60_000,
		env: {
			...process.env,
			MOLTIS_E2E_PORT: serverPort,
			MOLTIS_E2E_RUNTIME_DIR: path.join(repoRoot, "target", `e2e-runtime-${name}`),
		},
	};
}

const defaultWebServers = includeProject("default") && !skipDefaultProjects && isCi
	? defaultPorts.map((defaultPort) =>
			gatewayServer({
				baseURL,
				name: processDefaultShard ? `default-${processShardIndex}` : "default",
				port: defaultPort,
			}),
		)
	: includeProject("default") && !skipDefaultProjects
		? [
			{
				command: "./e2e/start-gateway.sh",
				cwd: __dirname,
				url: `${baseURL}/health`,
				reuseExistingServer: reuseExistingServer,
				timeout: 60_000,
				env: {
					...process.env,
					MOLTIS_E2E_PORT: port,
				},
			},
			]
		: [];
const ciIsolatedProjectWebServers = isCi
	? [
			includeProject("agents") ? gatewayServer({ baseURL: agentsBaseURL, name: "agents", port: agentsPort }) : null,
			includeProject("auth") ? gatewayServer({ baseURL: authBaseURL, name: "auth", port: authPort }) : null,
		].filter(Boolean)
	: [];

const webServer = defaultWebServers.concat(
	ciIsolatedProjectWebServers,
	[
		includeProject("onboarding")
			? {
					command: "./e2e/start-gateway-onboarding.sh",
					cwd: __dirname,
					url: `${onboardingBaseURL}/health`,
					reuseExistingServer: reuseExistingServer,
					timeout: 60_000,
					env: {
						...process.env,
						MOLTIS_E2E_ONBOARDING_PORT: onboardingPort,
					},
				}
			: null,
		includeProject("onboarding-auth")
			? {
					command: "./e2e/start-gateway-onboarding-auth.sh",
					cwd: __dirname,
					url: `${onboardingAuthBaseURL}/health`,
					reuseExistingServer: reuseExistingServer,
					timeout: 60_000,
					env: {
						...process.env,
						MOLTIS_E2E_ONBOARDING_AUTH_PORT: onboardingAuthPort,
					},
				}
			: null,
		includeProject("oauth")
			? {
					command: "./e2e/start-gateway-oauth.sh",
					cwd: __dirname,
					url: `${oauthBaseURL}/health`,
					reuseExistingServer: reuseExistingServer,
					timeout: 60_000,
					env: {
						...process.env,
						MOLTIS_E2E_OAUTH_PORT: oauthPort,
					},
				}
			: null,
		includeProject("onboarding-anthropic")
			? {
					command: "./e2e/start-gateway-onboarding-anthropic.sh",
					cwd: __dirname,
					url: `${onboardingAnthropicBaseURL}/health`,
					reuseExistingServer: reuseExistingServer,
					timeout: 60_000,
					env: {
						...process.env,
						MOLTIS_E2E_ONBOARDING_ANTHROPIC_PORT: onboardingAnthropicPort,
					},
				}
			: null,
	].filter(Boolean),
);

if (enableOpenAiLiveProject && includeProject("openai-live")) {
	webServer.push({
		command: "./e2e/start-gateway-openai-live.sh",
		cwd: __dirname,
		url: `${openaiLiveBaseURL}/health`,
		reuseExistingServer: reuseExistingServer,
		timeout: 60_000,
		env: {
			...process.env,
			MOLTIS_E2E_OPENAI_LIVE_PORT: openaiLivePort,
		},
	});
}

if (ollamaQwenLiveEnabled && includeProject("ollama-qwen-live")) {
	webServer.push({
		command: "./e2e/start-gateway-ollama-qwen-live.sh",
		cwd: __dirname,
		url: `${ollamaQwenLiveBaseURL}/health`,
		reuseExistingServer: reuseExistingServer,
		timeout: 60_000,
		env: {
			...process.env,
			MOLTIS_E2E_OLLAMA_QWEN_LIVE_PORT: ollamaQwenLivePort,
		},
	});
}

module.exports = defineConfig({
	testDir: "./e2e/specs",
	timeout: 60_000,
	expect: {
		timeout: 10_000,
	},
	fullyParallel: false,
	forbidOnly: !!process.env.CI,
	retries: 0,
	workers: 1,
	reporter: process.env.CI
		? [[path.join(__dirname, "e2e/ci-dot-reporter.cjs")], ["github"], ["html", { open: "never" }]]
		: [["list"], ["html", { open: "never" }]],
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
