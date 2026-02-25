// ── Crons page English strings ───────────────────────────────

export default {
	// ── Sidebar ─────────────────────────────────────────────
	sidebar: {
		cronJobs: "Cron Jobs",
		heartbeat: "Heartbeat",
	},

	// ── Heartbeat section ───────────────────────────────────
	heartbeat: {
		title: "Heartbeat",
		enable: "Enable",
		runNow: "Run Now",
		running: "Running\u2026",
		description: "Periodic AI check-in that monitors your environment and reports status.",
		inactiveLabel: "Heartbeat inactive:",
		blockedDisabled: "Heartbeat is disabled. Enable it to allow manual runs.",
		blockedNoPrompt:
			"Heartbeat is inactive because no prompt is configured. Add a custom prompt or write actionable content in HEARTBEAT.md.",
		blockedNoJob: "Heartbeat has no active cron job yet. Save the heartbeat settings to recreate it.",
		lastLabel: "Last:",
		nextLabel: "Next:",
		// Schedule
		scheduleTitle: "Schedule",
		intervalLabel: "Interval",
		modelLabel: "Model",
		modelDefaultPlaceholder: "(default: {{model}})",
		modelServerDefault: "(server default)",
		// Prompt
		promptTitle: "Prompt",
		customPromptLabel: "Custom Prompt (optional)",
		customPromptPlaceholder: "Leave blank to use default heartbeat prompt",
		customPromptHint:
			"Leave this empty to use HEARTBEAT.md in your workspace root. If that file exists but is empty/comments-only, heartbeat LLM runs are skipped to save tokens.",
		promptSourceLabel: "Effective prompt source:",
		promptSourceConfig: "config custom prompt",
		promptSourceMd: "HEARTBEAT.md",
		promptSourceDefault: "none (heartbeat inactive)",
		maxResponseCharsLabel: "Max Response Characters",
		// Active Hours
		activeHoursTitle: "Active Hours",
		activeHoursDescription: "Only run heartbeat during these hours.",
		startLabel: "Start",
		endLabel: "End",
		timezoneLabel: "Timezone",
		timezoneLocal: "Local ({{tz}})",
		// Sandbox
		sandboxTitle: "Sandbox",
		sandboxDescription: "Run heartbeat commands in an isolated container.",
		enableSandbox: "Enable sandbox",
		sandboxImageLabel: "Sandbox Image",
		sandboxImagePlaceholder: "Default image",
		sandboxSearchPlaceholder: "Search images\u2026",
		// Recent Runs
		recentRunsTitle: "Recent Runs",
		noRunsYet: "No runs yet.",
		// Token display
		tokenIn: "{{count}} in",
		tokenOut: "{{count}} out",
	},

	// ── Cron Jobs section ───────────────────────────────────
	jobs: {
		title: "Cron Jobs",
		addJob: "+ Add Job",
		noCronJobs: "No cron jobs configured.",
		// Status bar
		statusRunning: "Running",
		statusStopped: "Stopped",
		jobCount: "{{count}} job",
		jobCountPlural: "{{count}} jobs",
		enabledCount: "{{count}} enabled",
		nextRun: "next: {{time}}",
		// Table headers
		headerName: "Name",
		headerSchedule: "Schedule",
		headerNextRun: "Next Run",
		headerLastStatus: "Last Status",
		headerActions: "Actions",
		headerEnabled: "Enabled",
		// Actions
		edit: "Edit",
		run: "Run",
		history: "History",
		// Schedule formatting
		scheduleAt: "At {{time}}",
		scheduleEveryHours: "Every {{count}}h",
		scheduleEveryMinutes: "Every {{count}}m",
		scheduleEverySeconds: "Every {{count}}s",
		// Run history panel
		runHistoryTitle: "Run History: {{name}}",
		noRunsYet: "No runs yet.",
		// Delete confirmation
		deleteConfirm: "Delete job '{{name}}'?",
	},

	// ── Modal (Add / Edit) ──────────────────────────────────
	modal: {
		addJobTitle: "Add Job",
		editJobTitle: "Edit Job",
		nameLabel: "Name",
		namePlaceholder: "Job name",
		scheduleTypeLabel: "Schedule Type",
		scheduleAtOption: "At (one-shot)",
		scheduleEveryOption: "Every (interval)",
		scheduleCronOption: "Cron (expression)",
		everyPlaceholder: "Interval in seconds",
		cronPlaceholder: "*/5 * * * *",
		timezonePlaceholder: "Timezone (optional, e.g. Europe/Paris)",
		payloadTypeLabel: "Payload Type",
		systemEventOption: "System Event",
		agentTurnOption: "Agent Turn",
		messageLabel: "Message",
		messagePlaceholder: "Message text",
		sessionTargetLabel: "Session Target",
		targetIsolated: "Isolated",
		targetMain: "Main",
		deleteAfterRun: "Delete after run",
		create: "Create",
	},
};
