import type { VNode } from "preact";
import { targetChecked } from "../../typed-events";
import type { ManagedCandidate, RepositoryPreview } from "./types";

interface WarningBadgeProps {
	label: string;
}

export function WarningBadge({ label }: WarningBadgeProps): VNode {
	return (
		<span className="rounded-full bg-[var(--warn)] px-2 py-0.5 text-[0.65rem] font-medium text-black">{label}</span>
	);
}

interface CandidateCardProps {
	candidate: ManagedCandidate;
	checked: boolean;
	onChecked: (checked: boolean) => void;
}

function CandidateCard({ candidate, checked, onChecked }: CandidateCardProps): VNode {
	const invocation = candidate.url || [candidate.command, ...candidate.args].filter(Boolean).join(" ");
	return (
		<label
			className={`block rounded-lg border border-[var(--border)] bg-[var(--surface)] p-3 ${candidate.approvalBlocked ? "cursor-not-allowed opacity-70" : "cursor-pointer"}`}
		>
			<div className="flex items-start gap-3">
				<input
					type="checkbox"
					aria-label={`Select ${candidate.runtimeName}`}
					checked={checked}
					disabled={candidate.approvalBlocked}
					onChange={(event) => onChecked(targetChecked(event))}
					className="mt-1"
				/>
				<div className="min-w-0 flex-1">
					<div className="flex flex-wrap items-center gap-2">
						<span className="font-mono text-sm font-medium text-[var(--text-strong)]">{candidate.runtimeName}</span>
						<span className="rounded-full bg-[var(--surface2)] px-2 py-0.5 text-[0.65rem] text-[var(--muted)]">
							{candidate.transport}
						</span>
						{candidate.approved && <span className="text-xs text-[var(--ok)]">approved</span>}
						{candidate.approvalBlocked && <WarningBadge label="approval blocked" />}
						{candidate.warnings.map((warning, index) => (
							<WarningBadge key={`${warning}-${index}`} label={warning} />
						))}
					</div>
					<div className="mt-2 break-all rounded bg-[var(--surface2)] px-2 py-1.5 font-mono text-xs text-[var(--text)]">
						{invocation || "No command or URL"}
					</div>
					{candidate.approvalBlockReason && (
						<div className="mt-2 text-xs text-[var(--warn)]">{candidate.approvalBlockReason}</div>
					)}
					<div className="mt-2 grid gap-1 text-xs text-[var(--muted)] sm:grid-cols-2">
						<div>
							Identity: <code>{candidate.identity}</code>
						</div>
						<div>
							Cwd: <code>{candidate.cwd || "repository root"}</code>
						</div>
						<div>
							Environment names: <code>{candidate.envNames.join(", ") || "none"}</code>
						</div>
						<div>
							Header names: <code>{candidate.headerNames.join(", ") || "none"}</code>
						</div>
					</div>
				</div>
			</div>
		</label>
	);
}

interface RepositoryPreviewPanelProps {
	preview: RepositoryPreview;
	selected: Set<string>;
	onSelectionChange: (identity: string, checked: boolean) => void;
	children?: VNode | VNode[];
}

export function RepositoryPreviewPanel({
	preview,
	selected,
	onSelectionChange,
	children,
}: RepositoryPreviewPanelProps): VNode {
	return (
		<div className="mt-4 border-t border-[var(--border)] pt-4">
			<div className="flex flex-wrap items-center justify-between gap-2">
				<h4 className="text-sm font-medium text-[var(--text-strong)]">Repository preview</h4>
				<code className="break-all text-xs text-[var(--muted)]">Commit {preview.commit}</code>
			</div>
			{preview.warnings.length > 0 && (
				<section className="mt-3 flex flex-wrap gap-2" aria-label="Repository warnings">
					{preview.warnings.map((warning, index) => (
						<WarningBadge
							key={`${warning.kind}-${warning.sourceManifestPath}-${index}`}
							label={`${warning.kind}: ${warning.sourceManifestPath}`}
						/>
					))}
				</section>
			)}
			<div className="mt-3 grid gap-2">
				{preview.candidates.map((candidate) => (
					<CandidateCard
						key={candidate.identity}
						candidate={candidate}
						checked={selected.has(candidate.identity)}
						onChecked={(checked) => onSelectionChange(candidate.identity, checked)}
					/>
				))}
			</div>
			{children && <div className="mt-3 flex flex-wrap gap-2">{children}</div>}
		</div>
	);
}
