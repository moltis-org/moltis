// ── Skills page English strings ──────────────────────────────

export default {
	// ── Page header ─────────────────────────────────────────
	title: "Skills",
	refresh: "Refresh",
	emergencyDisable: "Emergency Disable",
	description: "SKILL.md-based skills discovered from project, personal, and installed paths.",
	howToWriteSkill: "How to write a skill?",

	// ── Emergency disable ───────────────────────────────────
	emergencyDisableConfirm: "Disable all third-party skills now?",
	disableAll: "Disable All",
	emergencyDisableFailed: "Emergency disable failed: {{error}}",
	disabledCount: "Disabled {{count}} skills",

	// ── Connection ──────────────────────────────────────────
	notConnected: "Not connected to gateway.",

	// ── Install ─────────────────────────────────────────────
	installPlaceholder: "owner/repo or full URL (e.g. anthropics/skills)",
	install: "Install",
	installing: "Installing\u2026",
	installingSource: "Installing {{source}}...",
	installMayTakeWhile: "This may take a while (download + scan).",
	installedSuccess: "Installed {{source}} ({{count}} skill{{s}})",
	failedGeneric: "Failed: {{error}}",

	// ── Loading ─────────────────────────────────────────────
	loadingSkills: "Loading skills\u2026",

	// ── Featured section ────────────────────────────────────
	featuredTitle: "Featured Repositories",

	// ── Repos section ───────────────────────────────────────
	reposTitle: "Installed Repositories",
	noRepos: "No repositories installed.",
	enabledCount: "{{enabled}}/{{total}} enabled",
	sha: "sha {{sha}}",
	sourceChanged: "source changed",
	orphanedOnDisk: "orphaned on disk",
	remove: "Remove",
	removing: "Removing...",
	searchSkillsIn: "Search skills in {{source}}\u2026",
	orphanedRepoHint: "Orphaned repo: reinstall to restore metadata",
	noMatchingSkills: "No matching skills.",

	// ── Enabled skills table ────────────────────────────────
	enabledTitle: "Enabled Skills",
	colName: "Name",
	colDescription: "Description",
	colSource: "Source",
	deletedSkill: "Deleted {{name}}",
	disabledSkill: "Disabled {{name}}",
	cannotDisableUnknownSource: "Cannot disable: unknown source for skill.",
	deleteSkillConfirm: 'Delete skill "{{name}}"? This removes the SKILL.md file.',
	disabling: "Disabling...",
	deleting: "Deleting...",

	// ── Skill detail panel ──────────────────────────────────
	protected: "Protected",
	protectedCannotDelete: "Skill {{name}} is protected and cannot be deleted from UI",
	trustAndEnable: "Trust & Enable",
	trustSkillConfirm: 'Trust skill "{{name}}" from {{source}}?',
	trustFailed: "Trust failed: {{error}}",
	failedToLoad: "Failed to load: {{error}}",
	skillMdSource: "SKILL.md source",

	// ── Metadata ────────────────────────────────────────────
	author: "Author: {{name}}",
	commit: "Commit:",
	commitAge: "Commit age: {{days}} day{{s}}",
	viewSource: "View source",
	allowedTools: "Allowed tools: {{tools}}",

	// ── Badges ──────────────────────────────────────────────
	blocked: "blocked",
	eligible: "eligible",
	noDeps: "no deps declared",
	untrusted: "untrusted",
	enabled: "enabled",

	// ── Missing deps ────────────────────────────────────────
	missing: "Missing: {{deps}}",
	installVia: "Install via {{kind}}",
	installDepConfirm:
		"Install dependency for {{name}}?\n\n{{preview}}\n\nOnly continue if you trust this skill and its source.",
	installedDep: "Installed dependency for {{name}}",
	installFailed: "Install failed: {{error}}",

	// ── Commit warning ──────────────────────────────────────
	recentCommitWarning: "Recent commit warning:",
	recentCommitMessage:
		"This skill was updated {{days}} day{{s}} ago. Treat recent updates as high risk and review diffs before trusting/enabling.",

	// ── Drift warning ───────────────────────────────────────
	driftWarning: "Source changed since last trust; review updates before enabling again.",

	// ── Security warning ────────────────────────────────────
	securityTitle: "\u26a0\ufe0f Skills run code on your machine \u2014 treat every skill as untrusted",
	securityIntro:
		"Skills are community-authored instructions that the AI agent follows <strong>with your full system privileges</strong>. Popularity or download count does not mean a skill is safe. A malicious skill can instruct the agent to:",
	threat1: "Execute arbitrary shell commands on your machine (install malware, cryptominers, backdoors)",
	threat2:
		"Read and exfiltrate sensitive data \u2014 SSH keys, API tokens, browser cookies, credentials, env variables",
	threat3: "Modify or delete files across your filesystem, including other projects",
	threat4: "Send your data to remote servers via curl/wget without your knowledge",
	securityReview:
		"Triple-check the source code of every skill before enabling it. Read the full SKILL.md and any scripts it references \u2014 these are the exact instructions the agent will execute on your behalf. Do not trust a skill just because it is popular, highly downloaded, or appears on a leaderboard.",
	securitySandbox:
		"With sandbox mode enabled (Docker, Apple Container, or cgroup), command execution is isolated and the damage a malicious skill can do is significantly limited.",
	dismiss: "Dismiss",
	disableAllThirdParty: "Disable all third-party skills",

	// ── Featured skill descriptions ─────────────────────────
	featuredOpenClaw: "Community skills from ClawdHub",
	featuredAnthropic: "Official Anthropic agent skills",
	featuredVercelAgent: "Vercel agent skills collection",
	featuredVercelSkills: "Vercel skills toolkit",
};
