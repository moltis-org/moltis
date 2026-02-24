// ── Sessions English strings ─────────────────────────────────

export default {
	// Welcome card
	welcome: {
		greeting: "Hello!",
		greetingWithName: "Hello, {{name}}!",
		noProviders: {
			title: "No LLM providers configured",
			description: "Configure at least one provider to start chatting.",
		},
	},

	// Session list
	list: {
		newSession: "New session",
		clearAll: "Clear",
		clearAllConfirm: "Delete {{count}} session? Main, Telegram and cron sessions will be kept.",
		clearAllConfirmPlural: "Delete {{count}} sessions? Main, Telegram and cron sessions will be kept.",
		clearing: "Clearing\u2026",
	},

	// Session header
	header: {
		fork: "Fork",
		forkTooltip: "Fork session",
		share: "Share",
		shareTooltip: "Share snapshot",
		clear: "Clear",
		clearTooltip: "Clear session",
		clearing: "Clearing\u2026",
		delete: "Delete",
		deleteTooltip: "Delete session",
		deleteConfirm: "Delete this session?",
		deleteWorktreeConfirm: "Worktree has uncommitted changes. Force delete?",
		renameTooltip: "Click to rename",
	},

	// Session item (list)
	item: {
		activeTelegram: "Active Telegram session",
		inactiveTelegram: "Telegram session (inactive)",
		fork: "fork@{{point}}",
	},

	// Session metadata (footer)
	meta: {
		tokenUsage: "{{inTokens}} in / {{outTokens}} out",
		modelProvider: "{{provider}} / {{model}}",
	},

	// Session actions
	actions: {
		clearFailed: "Clear failed",
	},

	// Share
	share: {
		linkCopied: "Share link copied",
		privateNotice: "Private link includes a key, share it only with trusted people",
		createFailed: "Failed to create share link",
	},

	// Search
	search: {
		noResults: "No results",
		placeholder: "Search sessions\u2026",
	},

	// Projects
	projects: {
		allSessions: "All sessions",
		noMatching: "No matching projects",
		filterTooltip: "Filter sessions by project",
	},
};
