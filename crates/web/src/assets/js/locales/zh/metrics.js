// ── Metrics page Chinese (Simplified) strings ───────────────

export default {
	// ── Page chrome ────────────────────────────────────────
	title: "监控",
	tabs: {
		overview: "概览",
		charts: "图表",
	},

	// ── Live indicator ─────────────────────────────────────
	live: "实时",

	// ── Loading & error states ─────────────────────────────
	loadingMetrics: "加载指标中\u2026",
	metricsDisabled: "指标未启用。在 moltis.toml 中设置 [metrics] enabled = true 来启用",

	// ── Empty states ───────────────────────────────────────
	noActivityTitle: "暂无活动",
	noActivityDescription: "开始使用 moltis 后，指标将显示在这里。试试发送消息或运行工具来查看数据。",
	collectingTitle: "数据收集中\u2026",
	collectingDescription: "收集到一些数据点后，历史图表将显示在这里。这通常需要大约 20\u201330 秒。",

	// ── Time range selector ────────────────────────────────
	timeRange: {
		fiveMin: "5 分钟",
		oneHour: "1 小时",
		twentyFourHours: "24 小时",
		sevenDays: "7 天",
	},

	// ── Section headings ───────────────────────────────────
	sections: {
		system: "系统",
		llmUsage: "LLM 用量",
		toolsMcp: "工具与 MCP",
		byProvider: "按供应商",
		prometheus: "Prometheus 端点",
	},

	// ── Metric card titles ─────────────────────────────────
	cards: {
		uptime: "运行时间",
		connectedClients: "已连接客户端",
		activeSessions: "活跃会话",
		httpRequests: "HTTP 请求",
		completions: "补全次数",
		inputTokens: "输入 Token",
		outputTokens: "输出 Token",
		cacheTokens: "缓存 Token",
		toolExecutions: "工具执行",
		toolsActive: "活跃工具",
		mcpToolCalls: "MCP 工具调用",
		mcpServers: "MCP 服务器",
	},

	// ── Metric card subtitles (interpolated) ───────────────
	errorsCount: "{{count}} 个错误",
	cacheRead: "读取：{{value}}",

	// ── Chart titles ───────────────────────────────────────
	charts: {
		tokenUsageTotal: "Token 用量（总计）",
		inputTokensByProvider: "按供应商的输入 Token",
		outputTokensByProvider: "按供应商的输出 Token",
		requests: "请求",
		connections: "连接",
		toolActivity: "工具活动",
	},

	// ── Chart series labels ────────────────────────────────
	series: {
		time: "时间",
		inputTokens: "输入 Token",
		outputTokens: "输出 Token",
		httpRequests: "HTTP 请求",
		llmCompletions: "LLM 补全",
		wsActive: "活跃 WebSocket",
		activeSessions: "活跃会话",
		toolExecutions: "工具执行",
		mcpCalls: "MCP 调用",
	},

	// ── Provider table headers ─────────────────────────────
	table: {
		provider: "供应商",
		completions: "补全次数",
		inputTokens: "输入 Token",
		outputTokens: "输出 Token",
		errors: "错误",
	},

	// ── Prometheus section ─────────────────────────────────
	prometheusDescription: "使用 Prometheus 抓取此端点或导入到 Grafana 进行高级可视化。",
};
