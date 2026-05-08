// ── Crons page Traditional Chinese (Taiwan) strings ─────────

export default {
	// ── Sidebar ─────────────────────────────────────────────
	sidebar: {
		cronJobs: "排程任務",
		heartbeat: "心跳檢查",
	},

	// ── Heartbeat section ───────────────────────────────────
	heartbeat: {
		title: "心跳檢查",
		enable: "啟用",
		runNow: "立即執行",
		running: "執行中\u2026",
		description: "由 AI 定期自動檢查環境並回報狀態。",
		inactiveLabel: "心跳檢查未啟用：",
		blockedDisabled: "心跳檢查已停用。請先啟用以允許手動執行。",
		blockedNoPrompt:
			"心跳檢查目前未啟用，因為尚未設定提示詞。請新增自訂提示詞，或在 HEARTBEAT.md 中撰寫實際的提示內容。",
		blockedNoJob: "心跳檢查尚未建立排程任務。請儲存心跳檢查設定以重新建立。",
		lastLabel: "上次：",
		nextLabel: "下次：",
		// Schedule
		scheduleTitle: "排程",
		intervalLabel: "間隔",
		modelLabel: "模型",
		modelDefaultPlaceholder: "（預設：{{model}}）",
		modelServerDefault: "（伺服器預設）",
		// Prompt
		promptTitle: "提示詞",
		customPromptLabel: "自訂提示詞（選填）",
		customPromptPlaceholder: "留空以使用預設心跳檢查提示詞",
		customPromptHint:
			"留空以使用工作區根目錄的 HEARTBEAT.md。如果該檔案存在但內容為空或僅含註解，心跳檢查 LLM 將跳過執行以節省 token。",
		promptSourceLabel: "生效的提示詞來源：",
		promptSourceConfig: "組態自訂提示詞",
		promptSourceMd: "HEARTBEAT.md",
		promptSourceDefault: "無（心跳檢查未啟用）",
		maxResponseCharsLabel: "回應字元數上限",
		// Active Hours
		activeHoursTitle: "有效時段",
		activeHoursDescription: "僅在這些時段內執行心跳檢查。",
		startLabel: "開始",
		endLabel: "結束",
		timezoneLabel: "時區",
		timezoneLocal: "本地（{{tz}}）",
		// Sandbox
		sandboxTitle: "沙盒",
		sandboxDescription: "在隔離的容器中執行心跳檢查指令。",
		enableSandbox: "啟用沙盒",
		sandboxImageLabel: "沙盒映像檔",
		sandboxImagePlaceholder: "預設映像檔",
		sandboxSearchPlaceholder: "搜尋映像檔\u2026",
		// Recent Runs
		recentRunsTitle: "最近的執行紀錄",
		noRunsYet: "尚無執行紀錄。",
		// Token display
		tokenIn: "{{count}} 輸入",
		tokenOut: "{{count}} 輸出",
	},

	// ── Cron Jobs section ───────────────────────────────────
	jobs: {
		title: "排程任務",
		addJob: "+ 新增任務",
		noCronJobs: "尚未設定排程任務。",
		// Status bar
		statusRunning: "執行中",
		statusStopped: "已停止",
		jobCount: "{{count}} 個任務",
		jobCountPlural: "{{count}} 個任務",
		enabledCount: "{{count}} 個已啟用",
		nextRun: "下次：{{time}}",
		// Table headers
		headerName: "名稱",
		headerSchedule: "排程",
		headerNextRun: "下次執行",
		headerLastStatus: "上次狀態",
		headerActions: "操作",
		headerEnabled: "已啟用",
		// Actions
		edit: "編輯",
		run: "執行",
		history: "歷史紀錄",
		// Schedule formatting
		scheduleAt: "於 {{time}}",
		scheduleEveryHours: "每 {{count}} 小時",
		scheduleEveryMinutes: "每 {{count}} 分鐘",
		scheduleEverySeconds: "每 {{count}} 秒",
		// Run history panel
		runHistoryTitle: "執行歷史紀錄：{{name}}",
		noRunsYet: "尚無執行紀錄。",
		// Delete confirmation
		deleteConfirm: "確定要刪除任務「{{name}}」嗎？",
	},

	// ── Modal (Add / Edit) ──────────────────────────────────
	modal: {
		addJobTitle: "新增任務",
		editJobTitle: "編輯任務",
		nameLabel: "名稱",
		namePlaceholder: "任務名稱",
		scheduleTypeLabel: "排程類型",
		scheduleAtOption: "定時（單次）",
		scheduleEveryOption: "間隔（週期）",
		scheduleCronOption: "Cron（運算式）",
		everyPlaceholder: "間隔秒數",
		cronPlaceholder: "*/5 * * * *",
		timezonePlaceholder: "時區（選填，例如 Europe/Paris）",
		payloadTypeLabel: "內容類型",
		systemEventOption: "系統事件",
		agentTurnOption: "AI 助理回合",
		messageLabel: "訊息",
		messagePlaceholder: "訊息內容",
		sessionTargetLabel: "工作階段目標",
		targetIsolated: "隔離",
		targetMain: "主要",
		deleteAfterRun: "執行後刪除",
		create: "建立",
	},
};
