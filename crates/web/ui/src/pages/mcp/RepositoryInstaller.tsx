import { useSignal } from "@preact/signals";
import type { VNode } from "preact";
import { CheckboxField, SelectField, TextField } from "../../components/forms";
import { RepositoryPreviewPanel } from "./RepositoryPreview";
import { mcpRepositoryRpc } from "./rpc";
import type { GitCredentialsResponse, RepositoryPreview, RepositoryRequest } from "./types";
import { expectedCandidates } from "./types";

interface RepositoryInstallerProps {
	credentials: GitCredentialsResponse;
	onChanged: () => Promise<void>;
	onMessage: (message: string, error?: boolean) => void;
}

export function RepositoryInstaller({ credentials, onChanged, onMessage }: RepositoryInstallerProps): VNode {
	const sourceKind = useSignal<"https" | "ssh" | "local">("https");
	const sourceValue = useSignal("");
	const privateHttps = useSignal(false);
	const credentialId = useSignal("");
	const sshTargetId = useSignal("");
	const alias = useSignal("");
	const repositoryId = useSignal("");
	const requestedRef = useSignal("HEAD");
	const preview = useSignal<RepositoryPreview | null>(null);
	const selected = useSignal<Set<string>>(new Set<string>());
	const busy = useSignal(false);

	function clearPreview(): void {
		preview.value = null;
		selected.value = new Set<string>();
	}

	function changeSourceKind(value: string): void {
		if (value !== "https" && value !== "ssh" && value !== "local") return;
		sourceKind.value = value;
		sourceValue.value = "";
		privateHttps.value = false;
		credentialId.value = "";
		sshTargetId.value = "";
		clearPreview();
	}

	function request(): RepositoryRequest | null {
		const common = {
			...(repositoryId.value.trim() ? { id: repositoryId.value.trim() } : {}),
			alias: alias.value.trim(),
			ref: requestedRef.value.trim() || "HEAD",
			discovery: "explicit" as const,
		};
		if (!(common.alias && sourceValue.value.trim())) return null;
		if (sourceKind.value === "https") {
			if (privateHttps.value && !credentialId.value) return null;
			return {
				...common,
				source: { kind: "https", url: sourceValue.value.trim(), private: privateHttps.value },
				...(privateHttps.value ? { httpsCredentialId: Number(credentialId.value) } : {}),
			};
		}
		if (sourceKind.value === "ssh") {
			if (!sshTargetId.value) return null;
			return {
				...common,
				source: { kind: "ssh", remote: sourceValue.value.trim() },
				sshTargetId: Number(sshTargetId.value),
			};
		}
		return { ...common, source: { kind: "local", path: sourceValue.value.trim() } };
	}

	async function loadPreview(): Promise<void> {
		const repository = request();
		if (!repository) {
			onMessage("Complete the source, alias, and required credential fields before previewing.", true);
			return;
		}
		busy.value = true;
		try {
			const result = await mcpRepositoryRpc<RepositoryPreview>("mcp.repositories.preview", repository);
			preview.value = result;
			selected.value = new Set(
				result.candidates.filter((candidate) => !candidate.approvalBlocked).map((candidate) => candidate.identity),
			);
		} catch (error) {
			onMessage(error instanceof Error ? error.message : "Repository preview failed", true);
		} finally {
			busy.value = false;
		}
	}

	function setCandidateSelected(identity: string, checked: boolean): void {
		const next = new Set(selected.value);
		if (checked) next.add(identity);
		else next.delete(identity);
		selected.value = next;
	}

	async function install(mode: "selected" | "all"): Promise<void> {
		const currentPreview = preview.value;
		if (!currentPreview) return;
		const repository = request();
		if (!repository) {
			clearPreview();
			onMessage("Repository input changed. Preview it again before installing.", true);
			return;
		}
		const candidates =
			mode === "all"
				? currentPreview.candidates
				: currentPreview.candidates.filter((candidate) => selected.value.has(candidate.identity));
		if (candidates.length === 0) return;
		busy.value = true;
		try {
			await mcpRepositoryRpc("mcp.repositories.install", {
				...repository,
				id: currentPreview.repository.id,
				expectedCommit: currentPreview.commit,
				selection: { mode, candidates: expectedCandidates(candidates) },
			});
			clearPreview();
			onMessage(
				`Imported ${candidates.length} managed server${candidates.length === 1 ? "" : "s"} disabled and unapproved.`,
			);
			await onChanged();
		} catch (error) {
			onMessage(error instanceof Error ? error.message : "Repository install failed", true);
		} finally {
			busy.value = false;
		}
	}

	const placeholder =
		sourceKind.value === "https"
			? "https://github.com/example/mcp-tools.git"
			: sourceKind.value === "ssh"
				? "git@github.com:example/mcp-tools.git"
				: "/srv/mcp-tools";
	const sourceLabel = sourceKind.value === "local" ? "Server-side local path" : "Repository source";
	const currentRequest = request();
	const selectedCount = selected.value.size;

	return (
		<section className="rounded-xl border border-[var(--border)] bg-[var(--surface2)] p-4 sm:p-5">
			<div className="mb-4">
				<h3 className="text-sm font-medium text-[var(--text-strong)]">Add managed repository</h3>
				<p className="mt-1 text-xs text-[var(--muted)]">
					Preview is mandatory. Imports are always disabled and unapproved until you explicitly approve them below.
				</p>
			</div>
			<div className="grid gap-x-4 sm:grid-cols-2">
				<SelectField
					label="Source type"
					value={sourceKind.value}
					onChange={changeSourceKind}
					options={[
						{ value: "https", label: "HTTPS" },
						{ value: "ssh", label: "SSH" },
						{ value: "local", label: "Server-side local path" },
					]}
				/>
				<TextField
					label={sourceLabel}
					value={sourceValue.value}
					onInput={(value) => {
						sourceValue.value = value;
						clearPreview();
					}}
					placeholder={placeholder}
					monospace
				/>
				<TextField
					label="Alias"
					value={alias.value}
					onInput={(value) => {
						alias.value = value;
						clearPreview();
					}}
					placeholder="company-tools"
				/>
				<TextField
					label="Repository id (optional)"
					value={repositoryId.value}
					onInput={(value) => {
						repositoryId.value = value;
						clearPreview();
					}}
					placeholder="company-tools-v1"
				/>
				<TextField
					label="Git ref"
					value={requestedRef.value}
					onInput={(value) => {
						requestedRef.value = value;
						clearPreview();
					}}
					monospace
				/>
				{sourceKind.value === "https" && (
					<div>
						<CheckboxField
							label="Private HTTPS repository"
							checked={privateHttps.value}
							onChange={(checked) => {
								privateHttps.value = checked;
								credentialId.value = "";
								clearPreview();
							}}
						/>
						{privateHttps.value && (
							<SelectField
								label="HTTPS credential"
								value={credentialId.value}
								onChange={(value) => {
									credentialId.value = value;
									clearPreview();
								}}
								options={[
									{ value: "", label: "Select credential" },
									...credentials.credentials.map((credential) => ({
										value: String(credential.id),
										label: `${credential.username}@${credential.host}`,
									})),
								]}
							/>
						)}
					</div>
				)}
				{sourceKind.value === "ssh" && (
					<SelectField
						label="Managed SSH target"
						value={sshTargetId.value}
						onChange={(value) => {
							sshTargetId.value = value;
							clearPreview();
						}}
						options={[
							{ value: "", label: "Select managed target" },
							...credentials.sshTargets.map((target) => ({
								value: String(target.id),
								label: `${target.label} (${target.target})${target.hasKnownHost ? " - pinned" : " - no pin"}`,
							})),
						]}
						help="Only metadata and host pin availability are shown; key and known_hosts contents stay hidden."
					/>
				)}
			</div>
			<button type="button" className="provider-btn" onClick={loadPreview} disabled={busy.value || !currentRequest}>
				{busy.value && !preview.value ? "Previewing..." : "Preview repository"}
			</button>
			{preview.value && (
				<RepositoryPreviewPanel
					preview={preview.value}
					selected={selected.value}
					onSelectionChange={setCandidateSelected}
				>
					<button
						type="button"
						className="provider-btn"
						onClick={() => install("selected")}
						disabled={busy.value || selectedCount === 0}
					>
						Install selected ({selectedCount})
					</button>
					<button
						type="button"
						className="provider-btn provider-btn-secondary"
						onClick={() => install("all")}
						disabled={busy.value}
					>
						Install all ({preview.value.candidates.length})
					</button>
				</RepositoryPreviewPanel>
			)}
		</section>
	);
}
