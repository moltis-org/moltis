// ── MCP page English strings ─────────────────────────────

export default {
	// ── Page header & intro ─────────────────────────────────
	title: "MCP",
	refresh: "Refresh",
	introTitle: "MCP (Model Context Protocol)",
	introDescription:
		"tools extend the AI agent with external capabilities \u2014 file access, web fetch, database queries, code search, and more.",
	flowAgent: "Agent",
	flowMoltis: "Moltis",
	flowLocalProcess: "Local MCP process",
	flowExternalApi: "External API",
	introDetail:
		"Each tool runs as a <strong>local process</strong> on your machine (spawned via npm/uvx). Moltis connects to it over stdio and the process makes outbound API calls on your behalf using your tokens. No data is sent to third-party MCP hosts.",

	// ── Security warning ────────────────────────────────────
	securityTitle: "\u26a0\ufe0f MCP servers run as local processes \u2014 review before enabling",
	securityPrivileges:
		"Each MCP server runs with <strong>your full system privileges</strong>. A malicious or compromised server can read your files, exfiltrate credentials, or execute arbitrary commands \u2014 just like any local process.",
	securityReview:
		"<strong>Triple-check the source code</strong> of any MCP server before enabling it. Only install servers from authors you trust, and keep them updated.",
	securityTokens:
		"Each enabled server also adds tool definitions to every chat session's context, consuming tokens. Only enable servers you actively need.",

	// ── Featured servers section ─────────────────────────────
	popularTitle: "Popular MCP Servers",
	browseAll: "Browse all servers on GitHub \u2192",
	configRequired: "config required",
	adding: "Adding\u2026",
	confirm: "Confirm",

	// ── Featured server descriptions ────────────────────────
	featured: {
		filesystemDesc: "Secure file operations with configurable access controls",
		filesystemHint: "Last arg is the allowed directory path",
		memoryDesc: "Knowledge graph-based persistent memory system",
		githubDesc: "GitHub API integration \u2014 repos, issues, PRs, code search",
		githubHint: "Requires a GitHub personal access token",
	},

	// ── Config form ─────────────────────────────────────────
	argumentsLabel: "Arguments",
	envVarsLabel: "Environment variables (KEY=VALUE per line)",

	// ── Install box (custom server) ─────────────────────────
	addCustomTitle: "Add Custom MCP Server",
	stdioLocal: "Stdio (local)",
	sseRemote: "SSE (remote)",
	commandLabel: "Command",
	commandPlaceholder: "npx -y mcp-remote https://mcp.example.com/mcp",
	serverUrlLabel: "Server URL",
	serverUrlPlaceholder: "https://mcp.example.com/mcp",
	nameLabel: "Name:",
	editableAfterAdding: "(editable after adding)",
	hideEnvVars: "Hide env vars",
	showEnvVars: "+ Environment variables",
	envVarsPlaceholder: "API_KEY=sk-...",

	// ── Server card ─────────────────────────────────────────
	edit: "Edit",
	restart: "Restart",
	toolCount: "{{count}} tool",
	toolCountPlural: "{{count}} tools",
	tokenEstimate: "~{{tokens}} tokens",
	loadingTools: "Loading tools\u2026",
	noTools: "No tools exposed by this server.",

	// ── Configured servers section ──────────────────────────
	configuredTitle: "Configured MCP Servers",
	noServersConfigured: "No MCP tools configured. Add one from the popular list above or enter a custom command.",
	loadingServers: "Loading MCP servers\u2026",

	// ── Toast messages ──────────────────────────────────────
	addedServer: 'Added MCP tool "{{name}}"',
	failedToAdd: 'Failed to add "{{name}}": {{error}}',
	failedGeneric: "Failed: {{error}}",
	restarted: 'Restarted "{{name}}"',
	updated: 'Updated "{{name}}"',
	failedToUpdate: "Failed to update: {{error}}",
	removed: 'Removed "{{name}}"',
	removeConfirm: 'This will stop and remove the "{{name}}" MCP tool. This action cannot be undone.',
};
