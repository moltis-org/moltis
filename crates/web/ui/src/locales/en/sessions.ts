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

	// Session list tabs
	tabs: {
		sessions: "Sessions",
		cron: "Cron",
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
		renameSessionTooltip: "Rename session",
		generateTitleTooltip: "Auto-generate title from conversation",
		generateTitleFailed: "Failed to auto-generate title",
		autoTitle: "Auto-title",
		deleteFailed: "Failed to delete session",
		stopFailed: "Failed to stop response",
		stopTooltip: "Stop generation",
		stopping: "Stopping\u2026",
		archive: "Archive",
		archiveTooltip: "Archive session",
		unarchive: "Unarchive",
		unarchiveTooltip: "Unarchive session",
		archiveFailed: "Failed to update archive state",
		markdownSaved: "Session saved as Markdown",
		markdownSaveFailed: "Failed to save session as Markdown",
		saveMarkdownTooltip: "Save as Markdown",
		saveMarkdownLabel: "Save .md",
		releaseChannel: "Release channel",
		releasing: "Releasing\u2026",
		releaseFailed: "Failed to release channel",
		attachedToNamedChannel: "Attached to {{channel}}. Runs without tools or private context until released.",
		attachedToChannel: "Attached to a channel. Runs without tools or private context until released.",
		releasedFromNamedChannel: "Released from {{channel}}. Tools and private context are available here again.",
		releasedFromChannel: "Released from its channel. Tools and private context are available here again.",
		agentSwitchFailed: "Failed to switch agent",
		nodeSwitchFailed: "Failed to switch node",
		defaultAgentSuffix: " (default)",
		switching: "Switching\u2026",
		local: "Local",
		sshTarget: "SSH: {{target}}",
		sessionAgent: "Session agent",
		sessionNode: "Session node",
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
