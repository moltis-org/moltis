// ── Metrics page English strings ─────────────────────────

export default {
	// ── Page chrome ────────────────────────────────────────
	title: "Monitoring",
	tabs: {
		overview: "Overview",
		charts: "Charts",
	},

	// ── Live indicator ─────────────────────────────────────
	live: "Live",

	// ── Loading & error states ─────────────────────────────
	loadingMetrics: "Loading metrics\u2026",
	metricsDisabled: "Metrics are not enabled. Enable them in moltis.toml with [metrics] enabled = true",

	// ── Empty states ───────────────────────────────────────
	noActivityTitle: "No activity yet",
	noActivityDescription:
		"Metrics will appear here once you start using moltis. Try sending a message or running a tool to see data.",
	collectingTitle: "Collecting data\u2026",
	collectingDescription:
		"Historical charts will appear here after a few data points are collected. This typically takes about 20\u201330 seconds.",

	// ── Time range selector ────────────────────────────────
	timeRange: {
		fiveMin: "5 min",
		oneHour: "1 hour",
		twentyFourHours: "24 hours",
		sevenDays: "7 days",
	},

	// ── Section headings ───────────────────────────────────
	sections: {
		system: "System",
		llmUsage: "LLM Usage",
		toolsMcp: "Tools & MCP",
		byProvider: "By Provider",
		prometheus: "Prometheus Endpoint",
	},

	// ── Metric card titles ─────────────────────────────────
	cards: {
		uptime: "Uptime",
		connectedClients: "Connected Clients",
		activeSessions: "Active Sessions",
		httpRequests: "HTTP Requests",
		completions: "Completions",
		inputTokens: "Input Tokens",
		outputTokens: "Output Tokens",
		cacheTokens: "Cache Tokens",
		toolExecutions: "Tool Executions",
		toolsActive: "Tools Active",
		mcpToolCalls: "MCP Tool Calls",
		mcpServers: "MCP Servers",
	},

	// ── Metric card subtitles (interpolated) ───────────────
	errorsCount: "{{count}} errors",
	cacheRead: "read: {{value}}",

	// ── Chart titles ───────────────────────────────────────
	charts: {
		tokenUsageTotal: "Token Usage (Total)",
		inputTokensByProvider: "Input Tokens by Provider",
		outputTokensByProvider: "Output Tokens by Provider",
		requests: "Requests",
		connections: "Connections",
		toolActivity: "Tool Activity",
	},

	// ── Chart series labels ────────────────────────────────
	series: {
		time: "Time",
		inputTokens: "Input Tokens",
		outputTokens: "Output Tokens",
		httpRequests: "HTTP Requests",
		llmCompletions: "LLM Completions",
		wsActive: "WebSocket Active",
		activeSessions: "Active Sessions",
		toolExecutions: "Tool Executions",
		mcpCalls: "MCP Calls",
	},

	// ── Provider table headers ─────────────────────────────
	table: {
		provider: "Provider",
		completions: "Completions",
		inputTokens: "Input Tokens",
		outputTokens: "Output Tokens",
		errors: "Errors",
	},

	// ── Prometheus section ─────────────────────────────────
	prometheusDescription: "Scrape this endpoint with Prometheus or import into Grafana for advanced visualization.",
};
