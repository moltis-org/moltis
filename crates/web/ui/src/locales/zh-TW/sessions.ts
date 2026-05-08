// ── Sessions Traditional Chinese (Taiwan) strings ───────────

export default {
	// Welcome card
	welcome: {
		greeting: "您好！",
		greetingWithName: "{{name}}，您好！",
		noProviders: {
			title: "尚未設定 LLM 供應商",
			description: "請先設定至少一個供應商才能開始對話。",
		},
	},

	// Session list tabs
	tabs: {
		sessions: "工作階段",
		cron: "排程",
	},

	// Session list
	list: {
		newSession: "新工作階段",
		clearAll: "清除",
		clearAllConfirm: "確定要刪除 {{count}} 個工作階段嗎？主要、Telegram 和排程工作階段將會保留。",
		clearAllConfirmPlural: "確定要刪除 {{count}} 個工作階段嗎？主要、Telegram 和排程工作階段將會保留。",
		clearing: "清除中\u2026",
	},

	// Session header
	header: {
		fork: "分叉",
		forkTooltip: "分叉工作階段",
		share: "分享",
		shareTooltip: "分享快照",
		clear: "清除",
		clearTooltip: "清除工作階段",
		clearing: "清除中\u2026",
		delete: "刪除",
		deleteTooltip: "刪除工作階段",
		deleteConfirm: "確定要刪除此工作階段嗎？",
		deleteWorktreeConfirm: "工作目錄有未提交的變更。確定要強制刪除嗎？",
		renameTooltip: "按一下以重新命名",
	},

	// Session item (list)
	item: {
		activeTelegram: "使用中的 Telegram 工作階段",
		inactiveTelegram: "Telegram 工作階段（非使用中）",
		fork: "fork@{{point}}",
	},

	// Session metadata (footer)
	meta: {
		tokenUsage: "{{inTokens}} 輸入 / {{outTokens}} 輸出",
		modelProvider: "{{provider}} / {{model}}",
	},

	// Session actions
	actions: {
		clearFailed: "清除失敗",
	},

	// Share
	share: {
		linkCopied: "分享連結已複製",
		privateNotice: "私密連結包含金鑰，請僅分享給信任的對象",
		createFailed: "建立分享連結失敗",
	},

	// Search
	search: {
		noResults: "沒有結果",
		placeholder: "搜尋工作階段\u2026",
	},

	// Projects
	projects: {
		allSessions: "所有工作階段",
		noMatching: "沒有符合的專案",
		filterTooltip: "依專案篩選工作階段",
	},
};
