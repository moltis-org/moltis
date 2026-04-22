// ── Skill source constants ───────────────────────────────────
// Mirrors moltis_skills::types::SkillSource on the Rust side.
// Uses a const object instead of an enum to produce cleaner JS output.

export const SkillSource = {
	Project: "project",
	Personal: "personal",
	Plugin: "plugin",
	Registry: "registry",
	Bundled: "bundled",
} as const;

export type SkillSourceValue = (typeof SkillSource)[keyof typeof SkillSource];

/** Sources that are stored as local files (can be deleted, not just disabled). */
export function isDiscoveredSource(source: string | undefined): boolean {
	return source === SkillSource.Personal || source === SkillSource.Project;
}

/** Whether a source string looks like a repo path (contains `/`). */
export function isRepoSource(source: string | undefined): boolean {
	return !!source?.includes("/");
}

// ── Bundled category display metadata ────────────────────────

export interface BundledCategory {
	name: string;
	count: number;
	enabled: boolean;
}

export const CATEGORY_META: Record<string, { icon: string; desc: string }> = {
	apple: { icon: "\uD83C\uDF4E", desc: "Apple ecosystem (Shortcuts, HomeKit)" },
	audio: { icon: "\uD83C\uDFB5", desc: "Audio processing and music" },
	"autonomous-ai-agents": { icon: "\uD83E\uDD16", desc: "Multi-agent orchestration" },
	creative: { icon: "\uD83C\uDFA8", desc: "Writing, art, and content creation" },
	"data-science": { icon: "\uD83D\uDCCA", desc: "Data analysis and visualization" },
	devops: { icon: "\u2699\uFE0F", desc: "Infrastructure, CI/CD, and deployment" },
	dogfood: { icon: "\uD83D\uDC36", desc: "Internal tooling and self-reference" },
	email: { icon: "\u2709\uFE0F", desc: "Email management and automation" },
	gaming: { icon: "\uD83C\uDFAE", desc: "Game development and gaming tools" },
	github: { icon: "\uD83D\uDC19", desc: "GitHub workflows and integrations" },
	media: { icon: "\uD83D\uDCF7", desc: "Image, video, and media processing" },
	messaging: { icon: "\uD83D\uDCAC", desc: "Chat platforms and messaging" },
	mlops: { icon: "\uD83E\uDDE0", desc: "ML training, fine-tuning, and deployment" },
	"note-taking": { icon: "\uD83D\uDCDD", desc: "Notes and knowledge management" },
	productivity: { icon: "\u26A1", desc: "Task management and workflows" },
	research: { icon: "\uD83D\uDD2C", desc: "Academic papers and web research" },
	"smart-home": { icon: "\uD83C\uDFE0", desc: "Home automation and IoT" },
	"social-media": { icon: "\uD83D\uDCF1", desc: "Social platform integrations" },
	"software-development": { icon: "\uD83D\uDCBB", desc: "Coding, testing, and dev tools" },
};

export function categoryLabel(name: string): string {
	return name
		.split("-")
		.map((w) => w.charAt(0).toUpperCase() + w.slice(1))
		.join(" ");
}
