// ── Projects page English strings ────────────────────────────

export default {
	title: "Repositories",
	description:
		"Projects bind sessions to a codebase directory. When a session is linked to a project, context files (CLAUDE.md, AGENTS.md) are loaded automatically and a custom system prompt can be injected. Enable auto-worktree to give each session its own git branch for isolated work.",
	autoDetectDescription:
		'<strong class="text-[var(--text)]">Auto-detect</strong> scans common directories under your home folder (<code class="font-mono text-xs">~/Projects</code>, <code class="font-mono text-xs">~/Developer</code>, <code class="font-mono text-xs">~/src</code>, <code class="font-mono text-xs">~/code</code>, <code class="font-mono text-xs">~/repos</code>, <code class="font-mono text-xs">~/workspace</code>, <code class="font-mono text-xs">~/dev</code>, <code class="font-mono text-xs">~/git</code>) and Superset worktrees (<code class="font-mono text-xs">~/.superset/worktrees</code>) for git repositories and adds them as projects.',
	clearAllHint: "Clear All only removes repository entries from Moltis, it does not delete anything from disk.",
	noProjectsConfigured: "No projects configured. Add a directory above or use auto-detect.",
	confirmClearAll:
		"Clear all repositories from Moltis? This only removes them from the list and does not delete files on disk.",
	confirmClearAllButton: "Clear all",
	autoDetect: "Auto-detect",
	detecting: "Detecting\u2026",
	clearAll: "Clear All",
	clearing: "Clearing\u2026",
	autoDetectTooltip: "Scan common locations for git repositories and add them as projects",
	clearAllTooltip: "Remove all repository entries from Moltis without deleting files on disk",
	pathInput: {
		directory: "Directory",
		placeholder: "/path/to/project",
	},
	badges: {
		auto: "auto",
		worktree: "worktree",
		setup: "setup",
		teardown: "teardown",
		image: "image",
	},
	card: {
		systemPromptPrefix: "System prompt: ",
		editProject: "Edit project",
		edit: "edit",
		removeProject: "Remove project",
	},
	editForm: {
		label: "Label",
		labelPlaceholder: "Project name",
		directory: "Directory",
		directoryPlaceholder: "/path/to/project",
		systemPrompt: "System prompt (optional)",
		systemPromptPlaceholder: "Extra instructions for the LLM when working on this project...",
		setupCommand: "Setup command",
		setupCommandPlaceholder: "e.g. pnpm install",
		teardownCommand: "Teardown command",
		teardownCommandPlaceholder: "e.g. docker compose down",
		branchPrefix: "Branch prefix",
		branchPrefixPlaceholder: "default: moltis",
		sandboxImage: "Sandbox image",
		sandboxImagePlaceholder: "Default (ubuntu:25.10)",
		autoWorktree: "Auto-create git worktree per session",
	},
};
