// ── MCP page Traditional Chinese (Taiwan) strings ───────────

export default {
	// ── Page header & intro ─────────────────────────────────
	title: "MCP",
	refresh: "重新整理",
	introTitle: "MCP (Model Context Protocol)",
	introDescription: "工具透過外部功能擴充 AI 助理 — 檔案存取、網頁擷取、資料庫查詢、程式碼搜尋等。",
	flowAgent: "AI 助理",
	flowMoltis: "Moltis",
	flowLocalProcess: "本機 MCP 處理程序",
	flowExternalApi: "外部 API",
	introDetail:
		"每個工具都會以<strong>本機處理程序</strong>的形式在您的裝置上執行（透過 npm/uvx 啟動）。Moltis 透過 stdio 連線，該處理程序會使用您的權杖代為發出對外 API 請求。不會將任何資料傳送到第三方 MCP 主機。",

	// ── Security warning ────────────────────────────────────
	securityTitle: "\u26a0\ufe0f MCP 伺服器會以本機處理程序執行 — 請在啟用前仔細檢查",
	securityPrivileges:
		"每個 MCP 伺服器都以<strong>您的完整系統權限</strong>執行。惡意或遭入侵的伺服器可以讀取檔案、竊取憑證或執行任意指令 — 就像任何本機處理程序一樣。",
	securityReview:
		"<strong>請務必仔細檢查</strong>任何 MCP 伺服器的原始碼。只安裝您信任的作者所開發的伺服器，並保持更新。",
	securityTokens:
		"啟用伺服器後，Moltis 會把工具定義加入每個對話工作階段的上下文，並消耗 token。請只啟用確實需要的伺服器。",

	// ── Featured servers section ─────────────────────────────
	popularTitle: "熱門 MCP 伺服器",
	browseAll: "在 GitHub 上瀏覽所有伺服器 \u2192",
	configRequired: "需要設定",
	adding: "新增中\u2026",
	confirm: "確認",

	// ── Featured server descriptions ────────────────────────
	featured: {
		filesystemDesc: "具有可設定存取控制的安全檔案操作",
		filesystemHint: "最後一個參數是允許存取的目錄路徑",
		memoryDesc: "以知識圖譜為基礎的持久記憶系統",
		githubDesc: "GitHub API 整合 — 儲存庫、議題、PR、程式碼搜尋",
		githubHint: "需要 GitHub 個人存取權杖",
	},

	// ── Config form ─────────────────────────────────────────
	argumentsLabel: "參數",
	envVarsLabel: "環境變數（每行一組 KEY=VALUE）",

	// ── Install box (custom server) ─────────────────────────
	addCustomTitle: "新增自訂 MCP 伺服器",
	stdioLocal: "Stdio（本機）",
	sseRemote: "SSE（遠端）",
	commandLabel: "指令",
	commandPlaceholder: "npx -y mcp-remote https://mcp.example.com/mcp",
	serverUrlLabel: "伺服器 URL",
	serverUrlPlaceholder: "https://mcp.example.com/mcp",
	nameLabel: "名稱：",
	editableAfterAdding: "（新增後可編輯）",
	hideEnvVars: "隱藏環境變數",
	showEnvVars: "+ 環境變數",
	envVarsPlaceholder: "API_KEY=sk-...",

	// ── Server card ─────────────────────────────────────────
	edit: "編輯",
	restart: "重新啟動",
	toolCount: "{{count}} 個工具",
	toolCountPlural: "{{count}} 個工具",
	tokenEstimate: "約 {{tokens}} 個 token",
	loadingTools: "載入工具中\u2026",
	noTools: "此伺服器未提供任何工具。",

	// ── Configured servers section ──────────────────────────
	configuredTitle: "已設定的 MCP 伺服器",
	noServersConfigured: "尚未設定 MCP 工具。請從上方的熱門清單中新增，或輸入自訂指令。",
	loadingServers: "載入 MCP 伺服器中\u2026",

	// ── Toast messages ──────────────────────────────────────
	addedServer: "已新增 MCP 工具「{{name}}」",
	failedToAdd: "新增「{{name}}」失敗：{{error}}",
	failedGeneric: "失敗：{{error}}",
	restarted: "已重新啟動「{{name}}」",
	updated: "已更新「{{name}}」",
	failedToUpdate: "更新失敗：{{error}}",
	removed: "已移除「{{name}}」",
	removeConfirm: "這將停止並移除「{{name}}」MCP 工具。此操作無法復原。",
};
