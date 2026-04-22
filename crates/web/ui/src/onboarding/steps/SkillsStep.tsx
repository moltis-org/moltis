// ── Skills step (bundled category selection) ─────────────────
//
// Lets users toggle bundled skill categories during onboarding.
// Categories map to top-level directories under crates/skills/src/assets/.

import type { VNode } from "preact";
import { useEffect, useState } from "preact/hooks";
import { sendRpc } from "../../helpers";
import { t } from "../../i18n";

// ── Types ───────────────────────────────────────────────────

interface BundledCategory {
	name: string;
	count: number;
	enabled: boolean;
}

// ── Category display names and descriptions ─────────────────

const CATEGORY_META: Record<string, { icon: string; desc: string }> = {
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

function categoryLabel(name: string): string {
	return name
		.split("-")
		.map((w) => w.charAt(0).toUpperCase() + w.slice(1))
		.join(" ");
}

// ── SkillsStep ──────────────────────────────────────────────

export function SkillsStep({ onNext, onBack }: { onNext: () => void; onBack?: (() => void) | null }): VNode {
	const [categories, setCategories] = useState<BundledCategory[]>([]);
	const [totalSkills, setTotalSkills] = useState(0);
	const [loading, setLoading] = useState(true);
	const [toggling, setToggling] = useState<string | null>(null);

	useEffect(() => {
		sendRpc("skills.bundled.categories", {}).then((res) => {
			if (res?.ok) {
				const payload = res.payload as { categories?: BundledCategory[]; total_skills?: number };
				setCategories(payload.categories || []);
				setTotalSkills(payload.total_skills || 0);
			}
			setLoading(false);
		});
	}, []);

	function toggle(cat: BundledCategory): void {
		if (toggling) return;
		const newEnabled = !cat.enabled;
		setToggling(cat.name);
		sendRpc("skills.bundled.toggle_category", { category: cat.name, enabled: newEnabled }).then((res) => {
			setToggling(null);
			if (res?.ok) {
				setCategories((prev) => prev.map((c) => (c.name === cat.name ? { ...c, enabled: newEnabled } : c)));
			}
		});
	}

	function enableAll(): void {
		const disabled = categories.filter((c) => !c.enabled);
		if (!disabled.length) return;
		Promise.all(
			disabled.map((c) => sendRpc("skills.bundled.toggle_category", { category: c.name, enabled: true })),
		).then(() => {
			setCategories((prev) => prev.map((c) => ({ ...c, enabled: true })));
		});
	}

	function disableAll(): void {
		const enabled = categories.filter((c) => c.enabled);
		if (!enabled.length) return;
		Promise.all(
			enabled.map((c) => sendRpc("skills.bundled.toggle_category", { category: c.name, enabled: false })),
		).then(() => {
			setCategories((prev) => prev.map((c) => ({ ...c, enabled: false })));
		});
	}

	const enabledCount = categories.filter((c) => c.enabled).length;
	const enabledSkillCount = categories.filter((c) => c.enabled).reduce((sum, c) => sum + c.count, 0);

	return (
		<div className="flex flex-col gap-4">
			<h2 className="text-lg font-medium text-[var(--text-strong)]">{t("onboarding:skills.title")}</h2>
			<p className="text-xs text-[var(--muted)] leading-relaxed">{t("onboarding:skills.description")}</p>

			{loading ? (
				<div className="flex items-center justify-center gap-2 py-8">
					<div className="inline-block w-5 h-5 border-2 border-[var(--border)] border-t-[var(--accent)] rounded-full animate-spin" />
					<span className="text-sm text-[var(--muted)]">{t("common:status.loading")}</span>
				</div>
			) : (
				<>
					<div className="flex items-center justify-between">
						<span className="text-xs text-[var(--muted)]">
							{enabledCount} of {categories.length} categories ({enabledSkillCount} of {totalSkills} skills)
						</span>
						<div className="flex gap-2">
							<button
								type="button"
								className="text-xs text-[var(--accent)] hover:underline cursor-pointer bg-transparent border-none p-0"
								onClick={enableAll}
							>
								{t("onboarding:skills.enableAll")}
							</button>
							<span className="text-xs text-[var(--muted)]">/</span>
							<button
								type="button"
								className="text-xs text-[var(--accent)] hover:underline cursor-pointer bg-transparent border-none p-0"
								onClick={disableAll}
							>
								{t("onboarding:skills.disableAll")}
							</button>
						</div>
					</div>

					<div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
						{categories.map((cat) => {
							const meta = CATEGORY_META[cat.name];
							const icon = meta?.icon || "\uD83D\uDCE6";
							const desc = meta?.desc || "";
							return (
								<button
									key={cat.name}
									type="button"
									onClick={() => toggle(cat)}
									disabled={toggling === cat.name}
									className={`flex items-start gap-3 p-3 rounded-md border text-left cursor-pointer transition-colors ${
										cat.enabled
											? "border-[var(--accent)] bg-[var(--accent-bg,rgba(var(--accent-rgb,59,130,246),0.08))]"
											: "border-[var(--border)] bg-[var(--surface)] opacity-60"
									}`}
								>
									<span className="text-lg shrink-0 mt-0.5">{icon}</span>
									<div className="flex-1 min-w-0">
										<div className="flex items-center gap-2">
											<span className="text-sm font-medium text-[var(--text-strong)]">{categoryLabel(cat.name)}</span>
											<span className="text-xs text-[var(--muted)]">({cat.count})</span>
										</div>
										{desc && <div className="text-xs text-[var(--muted)] mt-0.5">{desc}</div>}
									</div>
									<div className="shrink-0 mt-1">
										{cat.enabled ? (
											<span className="icon icon-check-circle text-[var(--accent)]" />
										) : (
											<span className="w-4 h-4 rounded-full border-2 border-[var(--border)] inline-block" />
										)}
									</div>
								</button>
							);
						})}
					</div>
				</>
			)}

			<div className="flex flex-wrap items-center gap-3 mt-1">
				{onBack && (
					<button type="button" className="provider-btn provider-btn-secondary" onClick={onBack}>
						{t("common:actions.back")}
					</button>
				)}
				<div className="flex-1" />
				<button type="button" className="provider-btn" onClick={onNext}>
					{t("common:actions.continue")}
				</button>
			</div>
		</div>
	);
}
