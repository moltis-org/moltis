// ── Metrics page Traditional Chinese (Taiwan) strings ───────

export default {
	// ── Page chrome ────────────────────────────────────────
	title: "監控",
	tabs: {
		overview: "總覽",
		charts: "圖表",
	},

	// ── Live indicator ─────────────────────────────────────
	live: "即時",

	// ── Loading & error states ─────────────────────────────
	loadingMetrics: "載入指標中\u2026",
	metricsDisabled: "指標功能未啟用。請在 moltis.toml 中以 [metrics] enabled = true 啟用",

	// ── Empty states ───────────────────────────────────────
	noActivityTitle: "尚無活動",
	noActivityDescription: "開始使用 moltis 後，指標將會顯示在此處。試著傳送一則訊息或執行一個工具來檢視資料。",
	collectingTitle: "正在收集資料\u2026",
	collectingDescription: "收集到幾個資料點後，歷史圖表將會顯示在此處。通常需要約 20\u201330 秒。",

	// ── Time range selector ────────────────────────────────
	timeRange: {
		fiveMin: "5 分鐘",
		oneHour: "1 小時",
		twentyFourHours: "24 小時",
		sevenDays: "7 天",
	},

	// ── Section headings ───────────────────────────────────
	sections: {
		system: "系統",
		llmUsage: "LLM 用量",
		toolsMcp: "工具與 MCP",
		byProvider: "依供應商",
		prometheus: "Prometheus 端點",
	},

	// ── Metric card titles ─────────────────────────────────
	cards: {
		uptime: "執行時間",
		connectedClients: "已連線的客戶端",
		activeSessions: "使用中的工作階段",
		httpRequests: "HTTP 請求",
		processMemory: "處理程序記憶體",
		completions: "完成次數",
		inputTokens: "輸入 Token",
		outputTokens: "輸出 Token",
		cacheTokens: "快取 Token",
		toolExecutions: "工具執行次數",
		toolsActive: "使用中的工具",
		mcpToolCalls: "MCP 工具呼叫",
		mcpServers: "MCP 伺服器",
	},

	// ── Metric card subtitles (interpolated) ───────────────
	errorsCount: "{{count}} 個錯誤",
	cacheRead: "讀取：{{value}}",

	// ── Chart titles ───────────────────────────────────────
	charts: {
		tokenUsageTotal: "Token 用量（總計）",
		inputTokensByProvider: "依供應商的輸入 Token",
		outputTokensByProvider: "依供應商的輸出 Token",
		requests: "請求",
		connections: "連線",
		memoryUsage: "記憶體用量（MiB）",
		toolActivity: "工具活動",
	},

	// ── Chart series labels ────────────────────────────────
	series: {
		time: "時間",
		inputTokens: "輸入 Token",
		outputTokens: "輸出 Token",
		httpRequests: "HTTP 請求",
		llmCompletions: "LLM 完成次數",
		wsActive: "WebSocket 使用中",
		activeSessions: "使用中的工作階段",
		processMemory: "處理程序記憶體",
		localLlamaCpp: "本機 llama.cpp",
		toolExecutions: "工具執行次數",
		mcpCalls: "MCP 呼叫",
	},

	// ── Provider table headers ─────────────────────────────
	table: {
		provider: "供應商",
		completions: "完成次數",
		inputTokens: "輸入 Token",
		outputTokens: "輸出 Token",
		errors: "錯誤",
	},

	// ── Prometheus section ─────────────────────────────────
	prometheusDescription: "使用 Prometheus 抓取此端點，或匯入 Grafana 以進行進階視覺化。",
};
