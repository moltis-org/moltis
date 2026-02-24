// ── Channels page English strings ────────────────────────────

export default {
	// ── Page & tabs ─────────────────────────────────────────
	title: "Channels",
	tabs: {
		channels: "Channels",
		senders: "Senders",
	},
	addTelegramBot: "+ Add Telegram Bot",

	// ── Channel card ────────────────────────────────────────
	card: {
		defaultName: "Telegram",
		unknownStatus: "unknown",
		editTitle: "Edit {{name}}",
		removeTitle: "Remove {{name}}",
		noActiveSession: "No active session",
		sessionInfo: "{{label}} ({{count}} msgs)",
		removeConfirm: "Remove {{name}}?",
		fallbackName: "channel",
	},

	// ── Empty states ────────────────────────────────────────
	empty: {
		noBotsConnected: "No Telegram bots connected.",
		addBotHint: 'Click "+ Add Telegram Bot" to connect one using a token from @BotFather.',
		noChannelsConfigured: "No channels configured.",
	},

	// ── Senders tab ─────────────────────────────────────────
	senders: {
		accountLabel: "Account:",
		noMessagesYet: "No messages received yet for this account.",
		colSender: "Sender",
		colUsername: "Username",
		colMessages: "Messages",
		colLastSeen: "Last Seen",
		colStatus: "Status",
		colAction: "Action",
		otpCopied: "OTP code copied",
		otpPrefix: "OTP: ",
		allowed: "Allowed",
		denied: "Denied",
		approve: "Approve",
		deny: "Deny",
	},

	// ── Allowlist input ─────────────────────────────────────
	allowlistPlaceholder: "Type a username and press Enter",

	// ── Add channel modal ───────────────────────────────────
	add: {
		modalTitle: "Add Telegram Bot",
		helpHeading: "How to create a Telegram bot",
		helpStep1: "1. Open {{link}} in Telegram",
		helpStep2: "2. Send /newbot and follow the prompts to choose a name and username",
		helpStep3: "3. Copy the bot token (looks like 123456:ABC-DEF...) and paste it below",
		helpSeeMore: "See the {{link}} for more details.",
		botFather: "@BotFather",
		telegramBotTutorial: "Telegram Bot Tutorial",
		botUsernameLabel: "Bot username",
		botUsernamePlaceholder: "e.g. my_assistant_bot",
		botTokenLabel: "Bot Token (from @BotFather)",
		botTokenPlaceholder: "123456:ABC-DEF...",
		connectingBtn: "Connecting\u2026",
		connectBtn: "Connect Bot",
		failedToConnect: "Failed to connect bot.",
	},

	// ── Edit channel modal ──────────────────────────────────
	edit: {
		modalTitle: "Edit Telegram Bot",
		saveChangesBtn: "Save Changes",
		failedToUpdate: "Failed to update bot.",
	},

	// ── Shared form labels ──────────────────────────────────
	form: {
		dmPolicyLabel: "DM Policy",
		dmPolicyOpen: "Open (anyone)",
		dmPolicyAllowlist: "Allowlist only",
		dmPolicyDisabled: "Disabled",
		mentionModeLabel: "Group Mention Mode",
		mentionModeMention: "Must @mention bot",
		mentionModeAlways: "Always respond",
		mentionModeNone: "Don't respond in groups",
		defaultModelLabel: "Default Model",
		modelDefault: "(default: {{model}})",
		modelServerDefault: "(server default)",
		dmAllowlistLabel: "DM Allowlist",
	},
};
