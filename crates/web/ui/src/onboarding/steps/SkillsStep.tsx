// ── Skills step (bundled category selection) ─────────────────
//
// Lets users toggle bundled skill categories during onboarding.
// Categories map to top-level directories under crates/skills/src/assets/.

import type { VNode } from "preact";
import { useEffect, useState } from "preact/hooks";
import { sendRpc } from "../../helpers";
import { t } from "../../i18n";
import { type BundledCategory, CATEGORY_META, categoryLabel } from "../../types/skill-source";

// ── SkillsStep ──────────────────────────────────────────────

export function SkillsStep({ onNext, onBack }: { onNext: () => void; onBack?: (() => void) | null }): VNode {
	const [categories, setCategories] = useState<BundledCategory[]>([]);
	const [totalSkills, setTotalSkills] = useState(0);
	const [loading, setLoading] = useState(true);
	const [busy, setBusy] = useState(false);

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
		if (busy) return;
		const newEnabled = !cat.enabled;
		setBusy(true);
		sendRpc("skills.bundled.toggle_category", { category: cat.name, enabled: newEnabled }).then((res) => {
			setBusy(false);
			if (res?.ok) {
				setCategories((prev) => prev.map((c) => (c.name === cat.name ? { ...c, enabled: newEnabled } : c)));
			}
		});
	}

	function bulkToggle(enabled: boolean): void {
		const targets = categories.filter((c) => c.enabled !== enabled);
		if (!targets.length || busy) return;
		setBusy(true);
		Promise.all(
			targets.map((c) =>
				sendRpc("skills.bundled.toggle_category", { category: c.name, enabled }).then((res) => ({
					name: c.name,
					ok: !!res?.ok,
				})),
			),
		).then((results) => {
			setBusy(false);
			const succeeded = new Set(results.filter((r) => r.ok).map((r) => r.name));
			if (succeeded.size > 0) {
				setCategories((prev) => prev.map((c) => (succeeded.has(c.name) ? { ...c, enabled } : c)));
			}
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
								disabled={busy}
								onClick={() => bulkToggle(true)}
							>
								{t("onboarding:skills.enableAll")}
							</button>
							<span className="text-xs text-[var(--muted)]">/</span>
							<button
								type="button"
								className="text-xs text-[var(--accent)] hover:underline cursor-pointer bg-transparent border-none p-0"
								disabled={busy}
								onClick={() => bulkToggle(false)}
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
									disabled={busy}
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
