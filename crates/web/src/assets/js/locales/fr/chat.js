// ── Chat UI English strings ─────────────────────────────────

export default {
	// ── Input area ───────────────────────────────────────────
	placeholder: "Type a message...",
	sendBtn: "Send",
	micTooltip: "Click to start recording",
	micStarting: "Démarrage du microphone...",
	micStopAndSend: "Cliquez pour arrêter et envoyer",
	voiceTranscribing: "Transcription...",
	voiceTranscribingMessage: "Transcription de la voix...",

	// ── Slash commands ───────────────────────────────────────
	slashClear: "Clear conversation history",
	slashCompact: "Summarize conversation to save tokens",
	slashContext: "Show session context and project info",

	// ── Message labels ───────────────────────────────────────
	viaChannel: "via {{label}}",
	provider: "Provider: {{name}}",
	allowed: "Allowed",
	denied: "Denied",
	expired: "expired",
	queued: "Queued",

	// ── Sandbox / MCP labels ─────────────────────────────────
	sandboxed: "sandboxed",
	sandboxDirect: "direct",
	sandboxDisabled: "disabled",
	sandboxUnavailable: "unavailable",
	sandboxDisabledHint:
		"Sandboxes are disabled on cloud deploys without a container runtime. Install on a VM with Docker or Apple Container to enable this feature.",
	sandboxToggleTooltip: "Toggle sandbox mode",
	sandboxImageTooltip: "Sandbox image",
	mcpEnabled: "MCP",
	mcpEnabledTooltip: "MCP tools enabled \u2014 click to disable for this session",
	mcpDisabled: "MCP off",
	mcpDisabledTooltip: "MCP tools disabled \u2014 click to enable for this session",
	mcpToggleTooltip: "Toggle MCP tools for this session",

	// ── Debug panel ──────────────────────────────────────────
	debugTooltip: "Show context debug info",
	debugLabel: "Debug",
	contextTooltip: "Show full LLM context (system prompt + history)",
	contextLabel: "Context",
	loadingContext: "Loading context\u2026",
	failedToLoadContext: "Failed to load context",
	buildingContext: "Building full context\u2026",
	failedToBuildContext: "Failed to build context",
	copyBtn: "Copy",

	// ── Context card ─────────────────────────────────────────
	context: {
		title: "Context",
		session: "Session",
		key: "Key",
		unknown: "unknown",
		messages: "Messages",
		model: "Model",
		defaultModel: "default",
		providerLabel: "Provider",
		label: "Label",
		toolSupport: "Tool Support",
		enabled: "Enabled",
		disabled: "Disabled",
		project: "Project",
		projectName: "Name",
		unnamed: "(unnamed)",
		directory: "Directory",
		systemPrompt: "System Prompt",
		charsCount: "{{count}} chars",
		contextFiles: "Context Files ({{count}})",
		noProject: "No project bound to this session",
		tools: "Tools",
		toolsDisabledNoSupport: "Tools disabled \u2014 model doesn't support tool calling",
		noTools: "No tools registered",
		skillsAndPlugins: "Skills & Plugins",
		skillsDisabledNoSupport: "Skills disabled \u2014 model doesn't support tool calling",
		pluginPrefix: "[Plugin] ",
		skillPrefix: "[Skill] ",
		noSkillsOrPlugins: "No skills or plugins enabled",
		mcpTools: "MCP Tools",
		mcpToolsDisabledNoSupport: "MCP tools disabled \u2014 model doesn't support tool calling",
		mcpToolsDisabledSession: "MCP tools disabled for this session",
		toolCount: "{{count}} tool",
		toolCount_plural: "{{count}} tools",
		noMcpTools: "No MCP tools running",
		sandbox: "Sandbox",
		backend: "Backend",
		mode: "Mode",
		scope: "Scope",
		workspaceMount: "Workspace Mount",
		image: "Image",
		container: "Container",
		tokenUsage: "Token Usage",
		input: "Input",
		output: "Output",
		total: "Total",
		contextLeft: "Context left",
		contextPct: "{{pct}}% of {{total}}",
	},

	// ── Compact card ─────────────────────────────────────────
	compact: {
		title: "Conversation compacted",
		beforeCompact: "Before compact",
		messages: "Messages",
		totalTokens: "Total tokens",
		contextUsage: "Context usage",
		usageFormat: "{{pct}}% of {{total}}",
		afterCompact: "After compact",
		summaryMessage: "1 (summary)",
		status: "Status",
		statusMessage: "Conversation history replaced with a summary",
	},

	// ── Warnings ─────────────────────────────────────────────
	toolsDisabledWarning:
		"Tools disabled \u2014 the current model doesn't support tool calling. Running in chat-only mode.",
	unknown: "inconnu",
	unknownError: "erreur inconnue",
	done: "terminé",
	compactingConversation: "Compacting conversation\u2026",
	autoCompactContextLimit: "Compression de la conversation (limite de contexte atteinte)\u2026",
	autoCompactFailed: "Échec de la compression automatique : {{error}}",
	compactFailed: "Compact failed",
	contextFailed: "Context failed",
	sandboxImageBuildStart: "Construction de l'image sandbox (installation des paquets)\u2026",
	sandboxImageReady: "Image sandbox prête : {{tag}}",
	sandboxImageCached: "Image sandbox déjà en cache : {{tag}}",
	sandboxImageBuildFailed: "Échec de construction de l'image sandbox : {{error}}",
	sandboxProvisionStart: "Provisionnement des paquets sandbox\u2026",
	sandboxProvisionDone: "Paquets sandbox provisionnés",
	sandboxProvisionFailed: "Échec du provisionnement sandbox : {{error}}",
	sandboxHostInstallStart: "Installation de {{count}} paquets sur l'hôte\u2026",
	sandboxHostInstalledCount: "{{count}} installés",
	sandboxHostAlreadyPresentCount: "{{count}} déjà présents",
	sandboxHostReady: "Paquets hôte prêts ({{details}})",
	sandboxHostInstallFailed: "Échec de l'installation des paquets hôte : {{error}}",
	browserContainer: "conteneur navigateur",
	browserImagePullStart:
		"Récupération de l'image du conteneur navigateur ({{image}})\u2026 Cela peut prendre quelques minutes au premier lancement.",
	browserImageReady: "Image du conteneur navigateur prête : {{image}}",
	browserImagePullFailed: "Échec du téléchargement de l'image du conteneur navigateur : {{error}}",
	downloadFailed: "Échec du téléchargement de {{modelName}} : {{error}}",
	downloadReady: "{{modelName}} prêt",
	downloadingModel: "Téléchargement de {{modelName}}\u2026",
	downloadProgressWithTotal: "{{downloaded}} / {{total}} MB",
	downloadProgressNoTotal: "{{downloaded}} MB",
	connectedToGateway: "Connecté à la passerelle moltis v{{version}} à {{time}}",
	statusHandshakeFailed: "échec de l'initialisation",
	handshakeFailed: "Initialisation échouée : {{reason}}",
	statusDisconnectedReconnecting: "déconnecté \u2014 reconnexion\u2026",
	statusConnecting: "connexion...",
	geolocationNotSupported: "La géolocalisation n'est pas prise en charge",
	renderError: "Render error: {{message}}",

	// ── Token bar ────────────────────────────────────────────
	tokenBar: {
		inLabel: " in / ",
		outLabel: " out \u00b7 ",
		tokensSuffix: " tokens",
		contextLeft: " \u00b7 Context left before auto-compact: {{pct}}%",
		toolsDisabled: " \u00b7 Tools: disabled",
	},

	// ── Share dialog ─────────────────────────────────────────
	share: {
		title: "Share session snapshot",
		hint: "A snapshot is frozen at this point, later chat messages stay private.",
		redactionWarning:
			"We do best-effort redaction for API keys and tokens in shared tool output, but always review before sharing.",
		publicLink: "Public link",
		publicBadge: "Open",
		privateLink: "Private link",
		privateBadge: "Key required",
		linkReady: "Share link ready",
		privateHint: "This is a private share link with an embedded key. Send it only to trusted people.",
		publicHint: "This snapshot is frozen at this point in time.",
		linkCopied: "Share link copied",
	},

	// ── Roles ────────────────────────────────────────────────
	roles: {
		you: "You",
		assistant: "Assistant",
		unknown: "unknown",
	},

	// ── Full context panel ───────────────────────────────────
	fullContext: {
		chars: "chars",
		toolCall: "tool call",
		toolCall_plural: "tool calls",
		id: "id",
		copied: "Copied!",
		messagesSummary: "{{count}} messages \xb7 system prompt {{systemChars}} chars \xb7 total {{totalChars}} chars",
	},

	// ── Queued messages ──────────────────────────────────────
	queuedMessages: {
		cancelAllTooltip: "Cancel all queued",
	},
};
