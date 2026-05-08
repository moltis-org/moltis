// ── Channels page Traditional Chinese (Taiwan) strings ──────

export default {
	// ── Page & tabs ─────────────────────────────────────────
	title: "頻道",
	tabs: {
		channels: "頻道",
		senders: "傳送者",
	},
	addTelegramBot: "+ 新增 Telegram 機器人",

	// ── Channel card ────────────────────────────────────────
	card: {
		defaultName: "Telegram",
		unknownStatus: "未知",
		editTitle: "編輯 {{name}}",
		removeTitle: "移除 {{name}}",
		noActiveSession: "沒有使用中的工作階段",
		sessionInfo: "{{label}}（{{count}} 則訊息）",
		removeConfirm: "確定要移除 {{name}} 嗎？",
		fallbackName: "頻道",
	},

	// ── Empty states ────────────────────────────────────────
	empty: {
		noBotsConnected: "尚未連線任何 Telegram 機器人。",
		addBotHint: "按一下「+ 新增 Telegram 機器人」，使用來自 @BotFather 的權杖進行連線。",
		noChannelsConfigured: "尚未設定任何頻道。",
	},

	// ── Senders tab ─────────────────────────────────────────
	senders: {
		accountLabel: "帳號：",
		noMessagesYet: "此帳號尚未收到任何訊息。",
		colSender: "傳送者",
		colUsername: "使用者名稱",
		colMessages: "訊息數",
		colLastSeen: "最後活動時間",
		colStatus: "狀態",
		colAction: "操作",
		otpCopied: "OTP 驗證碼已複製",
		otpPrefix: "OTP:",
		allowed: "已允許",
		denied: "已拒絕",
		approve: "核准",
		deny: "拒絕",
	},

	// ── Allowlist input ─────────────────────────────────────
	allowlistPlaceholder: "輸入使用者名稱後按 Enter",

	// ── Add channel modal ───────────────────────────────────
	add: {
		modalTitle: "新增 Telegram 機器人",
		helpHeading: "如何建立 Telegram 機器人",
		helpStep1: "1. 在 Telegram 中開啟 {{link}}",
		helpStep2: "2. 傳送 /newbot 並依提示選擇名稱與使用者名稱",
		helpStep3: "3. 複製機器人權杖（格式如 123456:ABC-DEF…）並貼到下方",
		helpSeeMore: "詳細資訊請參閱 {{link}}。",
		botFather: "@BotFather",
		telegramBotTutorial: "Telegram Bot 教學",
		botUsernameLabel: "機器人使用者名稱",
		botUsernamePlaceholder: "例如 my_assistant_bot",
		botTokenLabel: "機器人權杖（來自 @BotFather）",
		botTokenPlaceholder: "123456:ABC-DEF...",
		connectingBtn: "連線中\u2026",
		connectBtn: "連線機器人",
		failedToConnect: "連線機器人失敗。",
	},

	// ── Edit channel modal ──────────────────────────────────
	edit: {
		modalTitle: "編輯 Telegram 機器人",
		saveChangesBtn: "儲存變更",
		failedToUpdate: "更新機器人失敗。",
	},

	// ── Shared form labels ──────────────────────────────────
	form: {
		dmPolicyLabel: "私訊政策",
		dmPolicyOpen: "開放（所有人）",
		dmPolicyAllowlist: "僅限允許清單",
		dmPolicyDisabled: "已停用",
		mentionModeLabel: "群組提及模式",
		mentionModeMention: "需 @提及機器人才會回應",
		mentionModeAlways: "總是回應",
		mentionModeNone: "群組中不回應",
		defaultModelLabel: "預設模型",
		modelDefault: "（預設：{{model}}）",
		modelServerDefault: "（伺服器預設）",
		dmAllowlistLabel: "私訊允許清單",
	},
};
