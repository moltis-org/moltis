// ── MCP page Chinese (Simplified) strings ───────────────────

export default {
	// ── Page header & intro ─────────────────────────────────
	title: "MCP",
	refresh: "刷新",
	introTitle: "MCP（模型上下文协议）",
	introDescription: "工具通过外部能力扩展 AI 代理 \u2014 文件访问、网页获取、数据库查询、代码搜索等。",
	flowAgent: "代理",
	flowMoltis: "Moltis",
	flowLocalProcess: "本地 MCP 进程",
	flowExternalApi: "外部 API",
	introDetail:
		"每个工具在你的机器上作为 <strong>本地进程</strong> 运行（通过 npm/uvx 启动）。Moltis 通过 stdio 连接到它，进程使用你的令牌代你发起外部 API 调用。数据不会发送到第三方 MCP 主机。",

	// ── Security warning ────────────────────────────────────
	securityTitle: "\u26a0\ufe0f MCP 服务器作为本地进程运行 \u2014 启用前请仔细审查",
	securityPrivileges:
		"每个 MCP 服务器以 <strong>你的完整系统权限</strong> 运行。恶意或被入侵的服务器可以读取你的文件、窃取凭据或执行任意命令 \u2014 就像任何本地进程一样。",
	securityReview:
		"<strong>在启用任何 MCP 服务器之前，请仔细检查其源代码</strong>。仅安装来自你信任的作者的服务器，并保持更新。",
	securityTokens: "每个启用的服务器还会将工具定义添加到每个聊天会话的上下文中，消耗 token。仅启用你实际需要的服务器。",

	// ── Featured servers section ─────────────────────────────
	popularTitle: "热门 MCP 服务器",
	browseAll: "在 GitHub 上浏览所有服务器 \u2192",
	configRequired: "需要配置",
	adding: "添加中\u2026",
	confirm: "确认",

	// ── Featured server descriptions ────────────────────────
	featured: {
		filesystemDesc: "具有可配置访问控制的安全文件操作",
		filesystemHint: "最后一个参数是允许访问的目录路径",
		memoryDesc: "基于知识图谱的持久化记忆系统",
		githubDesc: "GitHub API 集成 \u2014 仓库、议题、PR、代码搜索",
		githubHint: "需要 GitHub 个人访问令牌",
	},

	// ── Config form ─────────────────────────────────────────
	argumentsLabel: "参数",
	envVarsLabel: "环境变量（每行一个 KEY=VALUE）",

	// ── Install box (custom server) ─────────────────────────
	addCustomTitle: "添加自定义 MCP 服务器",
	stdioLocal: "Stdio（本地）",
	sseRemote: "SSE（远程）",
	commandLabel: "命令",
	commandPlaceholder: "npx -y mcp-remote https://mcp.example.com/mcp",
	serverUrlLabel: "服务器 URL",
	serverUrlPlaceholder: "https://mcp.example.com/mcp",
	nameLabel: "名称：",
	editableAfterAdding: "（添加后可编辑）",
	hideEnvVars: "隐藏环境变量",
	showEnvVars: "+ 环境变量",
	envVarsPlaceholder: "API_KEY=sk-...",

	// ── Server card ─────────────────────────────────────────
	edit: "编辑",
	restart: "重启",
	toolCount: "{{count}} 个工具",
	toolCountPlural: "{{count}} 个工具",
	tokenEstimate: "约 {{tokens}} tokens",
	loadingTools: "加载工具中\u2026",
	noTools: "此服务器未暴露任何工具。",

	// ── Configured servers section ──────────────────────────
	configuredTitle: "已配置的 MCP 服务器",
	noServersConfigured: "未配置 MCP 工具。从上方热门列表中添加或输入自定义命令。",
	loadingServers: "加载 MCP 服务器中\u2026",

	// ── Toast messages ──────────────────────────────────────
	addedServer: '已添加 MCP 工具 "{{name}}"',
	failedToAdd: '添加 "{{name}}" 失败：{{error}}',
	failedGeneric: "失败：{{error}}",
	restarted: '已重启 "{{name}}"',
	updated: '已更新 "{{name}}"',
	failedToUpdate: "更新失败：{{error}}",
	removed: '已移除 "{{name}}"',
	removeConfirm: '这将停止并移除 MCP 工具 "{{name}}"。此操作无法撤销。',
};
