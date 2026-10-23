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
		renameSessionTooltip: "Renommer la session",
		generateTitleTooltip: "Générer automatiquement un titre à partir de la conversation",
		generateTitleFailed: "Échec de la génération automatique du titre",
		autoTitle: "Titre automatique",
		deleteFailed: "Échec de la suppression de la session",
		stopFailed: "Échec de l'arrêt de la réponse",
		stopTooltip: "Arrêter la génération",
		stopping: "Arrêt en cours\u2026",
		archive: "Archiver",
		archiveTooltip: "Archiver la session",
		unarchive: "Désarchiver",
		unarchiveTooltip: "Désarchiver la session",
		archiveFailed: "Échec de la mise à jour de l'état d'archivage",
		markdownSaved: "Session enregistrée au format Markdown",
		markdownSaveFailed: "Échec de l'enregistrement de la session au format Markdown",
		saveMarkdownTooltip: "Enregistrer au format Markdown",
		saveMarkdownLabel: "Enregistrer en .md",
		releaseChannel: "Détacher du canal",
		releasing: "Détachement en cours\u2026",
		releaseFailed: "Échec du détachement du canal",
		attachedToNamedChannel:
			"Rattachée à {{channel}}. Fonctionne sans outils ni contexte privé tant qu'elle y est rattachée.",
		attachedToChannel: "Rattachée à un canal. Fonctionne sans outils ni contexte privé tant qu'elle y est rattachée.",
		releasedFromNamedChannel:
			"Détachée de {{channel}}. Les outils et le contexte privé sont à nouveau disponibles ici.",
		releasedFromChannel: "Détachée de son canal. Les outils et le contexte privé sont à nouveau disponibles ici.",
		agentSwitchFailed: "Échec du changement d'agent",
		nodeSwitchFailed: "Échec du changement de nœud",
		defaultAgentSuffix: " (par défaut)",
		switching: "Changement en cours\u2026",
		local: "Local",
		sshTarget: "SSH: {{target}}",
		sessionAgent: "Agent de la session",
		sessionNode: "Nœud de la session",
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
