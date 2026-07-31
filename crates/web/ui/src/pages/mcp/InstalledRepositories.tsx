import { useSignal } from "@preact/signals";
import type { VNode } from "preact";
import { targetChecked } from "../../typed-events";
import { requestConfirm } from "../../ui";
import { RepositoryPreviewPanel, WarningBadge } from "./RepositoryPreview";
import { mcpRepositoryRpc } from "./rpc";
import type { InstalledRepository, ManagedCandidate, RepositoryPreview } from "./types";
import { expectedCandidates, sourceDescription } from "./types";

interface InstalledRepositoriesProps {
	repositories: InstalledRepository[];
	onChanged: () => Promise<void>;
	onMessage: (message: string, error?: boolean) => void;
}

interface RepositoryCardProps {
	installed: InstalledRepository;
	onChanged: () => Promise<void>;
	onMessage: (message: string, error?: boolean) => void;
}

function RepositoryCard({ installed, onChanged, onMessage }: RepositoryCardProps): VNode {
	const updatePreview = useSignal<RepositoryPreview | null>(null);
	const approvalSelection = useSignal<Set<string>>(
		new Set(installed.servers.filter((server) => !server.managed?.approval_blocked).map((server) => server.name)),
	);
	const busy = useSignal(false);
	const repository = installed.repository;
	const approvedCount = installed.servers.filter((server) => server.managed?.approved).length;
	const enabledCount = installed.servers.filter((server) => server.enabled).length;

	function installedCandidates(): ManagedCandidate[] {
		return installed.servers.map((server) => ({
			runtimeName: server.name,
			identity: server.name,
			digest: "",
			transport: server.transport,
			command: "",
			args: [],
			envNames: [],
			headerNames: [],
			approved: Boolean(server.managed?.approved),
			approvalBlocked: Boolean(server.managed?.approval_blocked),
			approvalBlockReason: server.managed?.approval_block_reason,
			warnings: server.managed?.warning_kinds || [],
		}));
	}

	function selectionChanged(identity: string, checked: boolean): void {
		const next = new Set(approvalSelection.value);
		if (checked) next.add(identity);
		else next.delete(identity);
		approvalSelection.value = next;
	}

	async function previewUpdate(): Promise<void> {
		busy.value = true;
		try {
			const result = await mcpRepositoryRpc<RepositoryPreview>("mcp.repositories.update.preview", {
				id: repository.id,
			});
			updatePreview.value = result;
		} catch (error) {
			onMessage(error instanceof Error ? error.message : "Update preview failed", true);
		} finally {
			busy.value = false;
		}
	}

	async function applyUpdate(): Promise<void> {
		const preview = updatePreview.value;
		if (!preview) return;
		busy.value = true;
		try {
			await mcpRepositoryRpc("mcp.repositories.update.apply", {
				id: repository.id,
				expectedCommit: preview.commit,
				candidates: expectedCandidates(preview.candidates),
			});
			updatePreview.value = null;
			onMessage("Repository update applied. Changed and added servers are disabled and unapproved.");
			await onChanged();
		} catch (error) {
			onMessage(error instanceof Error ? error.message : "Repository update failed", true);
		} finally {
			busy.value = false;
		}
	}

	async function approve(mode: "selected" | "all", enable: boolean): Promise<void> {
		if (!installed.activeCommit) return;
		busy.value = true;
		try {
			const exactPreview = await mcpRepositoryRpc<RepositoryPreview>("mcp.repositories.update.preview", {
				id: repository.id,
			});
			if (exactPreview.commit !== installed.activeCommit) {
				updatePreview.value = exactPreview;
				throw new Error("The repository ref has changed. Review and apply the update before approving servers.");
			}
			const installedNames = new Set(installed.servers.map((server) => server.name));
			const candidates = exactPreview.candidates.filter((candidate) => {
				if (!installedNames.has(candidate.runtimeName)) return false;
				return !candidate.approvalBlocked && (mode === "all" || approvalSelection.value.has(candidate.runtimeName));
			});
			if (candidates.length === 0) throw new Error("Select at least one managed server to approve.");
			await mcpRepositoryRpc("mcp.managed.approve", {
				id: repository.id,
				expectedCommit: installed.activeCommit,
				selection: { mode, candidates: expectedCandidates(candidates) },
				enable,
			});
			onMessage(enable ? "Managed servers approved and enabled." : "Managed servers approved without enabling.");
			await onChanged();
		} catch (error) {
			onMessage(error instanceof Error ? error.message : "Approval failed", true);
		} finally {
			busy.value = false;
		}
	}

	async function rollback(): Promise<void> {
		if (!installed.previousCommit) return;
		const confirmed = await requestConfirm(`Roll back ${repository.alias} to ${installed.previousCommit}?`, {
			confirmLabel: "Rollback",
			danger: true,
		});
		if (!confirmed) return;
		busy.value = true;
		try {
			await mcpRepositoryRpc("mcp.repositories.rollback", {
				id: repository.id,
				expectedCommit: installed.previousCommit,
			});
			onMessage("Repository rolled back. Reconciled servers require approval again.");
			await onChanged();
		} catch (error) {
			onMessage(error instanceof Error ? error.message : "Rollback failed", true);
		} finally {
			busy.value = false;
		}
	}

	async function remove(): Promise<void> {
		const confirmed = await requestConfirm(
			`Remove managed repository ${repository.alias} and all servers it owns? Local source checkouts are not deleted.`,
			{ confirmLabel: "Remove", danger: true },
		);
		if (!confirmed) return;
		busy.value = true;
		try {
			await mcpRepositoryRpc("mcp.repositories.remove", { id: repository.id });
			onMessage("Managed repository removed.");
			await onChanged();
		} catch (error) {
			onMessage(error instanceof Error ? error.message : "Repository removal failed", true);
		} finally {
			busy.value = false;
		}
	}

	const candidates = installedCandidates();
	const selectedCount = approvalSelection.value.size;

	return (
		<article className="rounded-xl border border-[var(--border)] bg-[var(--surface2)] p-4 sm:p-5">
			<div className="flex flex-wrap items-start justify-between gap-3">
				<div className="min-w-0">
					<div className="flex flex-wrap items-center gap-2">
						<h4 className="text-base font-medium text-[var(--text-strong)]">{repository.alias}</h4>
						<span className="rounded-full bg-[var(--surface)] px-2 py-0.5 font-mono text-[0.65rem] text-[var(--muted)]">
							{repository.id}
						</span>
					</div>
					<div className="mt-1 break-all font-mono text-xs text-[var(--muted)]">
						{sourceDescription(repository.source)}
					</div>
					<div className="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-xs text-[var(--muted)]">
						<span>Ref: {repository.ref}</span>
						<span>Active: {installed.activeCommit || "none"}</span>
						<span>Previous: {installed.previousCommit || "none"}</span>
					</div>
				</div>
				<div className="flex flex-wrap gap-2 text-xs">
					<span className="rounded-full bg-[var(--surface)] px-2 py-1 text-[var(--text)]">
						Approved {approvedCount}/{installed.servers.length}
					</span>
					<span className="rounded-full bg-[var(--surface)] px-2 py-1 text-[var(--text)]">
						Enabled {enabledCount}/{installed.servers.length}
					</span>
				</div>
			</div>
			<div className="mt-4 grid gap-2">
				{candidates.map((candidate) => {
					const server = installed.servers.find((item) => item.name === candidate.runtimeName);
					return (
						<label
							key={candidate.runtimeName}
							className="flex cursor-pointer flex-wrap items-center gap-2 rounded-lg border border-[var(--border)] bg-[var(--surface)] p-3"
						>
							<input
								type="checkbox"
								aria-label={`Select installed ${candidate.runtimeName}`}
								checked={approvalSelection.value.has(candidate.runtimeName)}
								disabled={candidate.approvalBlocked}
								onChange={(event) => selectionChanged(candidate.runtimeName, targetChecked(event))}
							/>
							<span className="font-mono text-sm text-[var(--text-strong)]">{candidate.runtimeName}</span>
							<span className="text-xs text-[var(--muted)]">{candidate.transport}</span>
							<span className={`text-xs ${server?.managed?.approved ? "text-[var(--ok)]" : "text-[var(--warn)]"}`}>
								{server?.managed?.approved ? "approved" : "unapproved"}
							</span>
							<span className={`text-xs ${server?.enabled ? "text-[var(--ok)]" : "text-[var(--muted)]"}`}>
								{server?.enabled ? "enabled" : "disabled"}
							</span>
							{candidate.warnings.map((warning) => (
								<WarningBadge key={warning} label={warning} />
							))}
							{candidate.approvalBlocked && <WarningBadge label="approval blocked" />}
							<span className="ml-auto text-xs text-[var(--muted)]">
								Managed structure: edit through repository update
							</span>
						</label>
					);
				})}
			</div>
			<div className="mt-4 flex flex-wrap gap-2">
				<button
					type="button"
					className="provider-btn"
					onClick={() => approve("selected", false)}
					disabled={busy.value || selectedCount === 0}
				>
					Approve selected
				</button>
				<button
					type="button"
					className="provider-btn"
					onClick={() => approve("selected", true)}
					disabled={busy.value || selectedCount === 0}
				>
					Approve and enable selected
				</button>
				<button
					type="button"
					className="provider-btn provider-btn-secondary"
					onClick={() => approve("all", true)}
					disabled={busy.value}
				>
					Approve and enable all
				</button>
				<button
					type="button"
					className="provider-btn provider-btn-secondary"
					onClick={previewUpdate}
					disabled={busy.value}
				>
					Preview update
				</button>
				<button
					type="button"
					className="provider-btn provider-btn-secondary"
					onClick={rollback}
					disabled={busy.value || !installed.previousCommit}
				>
					Rollback
				</button>
				<button type="button" className="provider-btn provider-btn-danger" onClick={remove} disabled={busy.value}>
					Remove repository
				</button>
			</div>
			{updatePreview.value && (
				<div className="mt-4">
					<h5 className="text-sm font-medium text-[var(--text-strong)]">Update reconciliation</h5>
					<div className="mt-2 grid gap-2 text-xs sm:grid-cols-4">
						{(["added", "updated", "removed", "unchanged"] as const).map((kind) => (
							<div key={kind} className="rounded-lg bg-[var(--surface)] p-2 text-[var(--muted)]">
								<strong className="block capitalize text-[var(--text-strong)]">{kind}</strong>
								{updatePreview.value?.diff?.[kind].join(", ") || "none"}
							</div>
						))}
					</div>
					<RepositoryPreviewPanel
						preview={updatePreview.value}
						selected={new Set(updatePreview.value.candidates.map((candidate) => candidate.identity))}
						onSelectionChange={() => undefined}
					>
						<button type="button" className="provider-btn" onClick={applyUpdate} disabled={busy.value}>
							Apply update
						</button>
						<button
							type="button"
							className="provider-btn provider-btn-secondary"
							onClick={() => (updatePreview.value = null)}
						>
							Clear update preview
						</button>
					</RepositoryPreviewPanel>
				</div>
			)}
		</article>
	);
}

export function InstalledRepositories({ repositories, onChanged, onMessage }: InstalledRepositoriesProps): VNode {
	return (
		<section>
			<h3 className="mb-2 text-sm font-medium text-[var(--text-strong)]">Installed managed repositories</h3>
			{repositories.length === 0 ? (
				<div className="rounded-xl border border-dashed border-[var(--border)] p-4 text-sm text-[var(--muted)]">
					No managed repositories installed.
				</div>
			) : (
				<div className="grid gap-3">
					{repositories.map((repository) => (
						<RepositoryCard
							key={repository.repository.id}
							installed={repository}
							onChanged={onChanged}
							onMessage={onMessage}
						/>
					))}
				</div>
			)}
		</section>
	);
}
