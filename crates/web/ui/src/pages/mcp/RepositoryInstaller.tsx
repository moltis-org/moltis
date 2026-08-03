import { useSignal } from "@preact/signals";
import type { VNode } from "preact";
import { CheckboxField, SelectField, TextField } from "../../components/forms";
import { RepositoryPreviewPanel } from "./RepositoryPreview";
import { mcpRepositoryRpc } from "./rpc";
import type {
	GitCredentialsResponse,
	GitHttpsCredential,
	ManagedCandidate,
	RepositoryPreview,
	RepositoryRequest,
	SshTargetMetadata,
} from "./types";
import { expectedCandidates } from "./types";

function normalizeHttpsSource(value: string): string {
	const source = value.trim();
	const parts = source.split("/");
	if (parts.length !== 2 || parts.some((part) => !part || /[^A-Za-z0-9_.-]/.test(part))) return source;
	const repository = parts[1]?.replace(/\.git$/, "");
	return `https://github.com/${parts[0]}/${repository}.git`;
}

function repositoryName(value: string): string {
	const source = value.trim().replace(/\/$/, "");
	const path = source.startsWith("ssh://")
		? (() => {
				try {
					return new URL(source).pathname;
				} catch {
					return source;
				}
			})()
		: (source.split(":").at(-1) ?? source);
	const name = (path.split("/").at(-1) ?? "").replace(/\.git$/, "");
	return name
		.replace(/[^A-Za-z0-9._-]+/g, "-")
		.replace(/^[^A-Za-z0-9]+/, "")
		.slice(0, 64);
}

function httpsAuthority(value: string): string | null {
	try {
		const url = new URL(normalizeHttpsSource(value));
		return url.protocol === "https:" ? url.host.toLowerCase() : null;
	} catch {
		return null;
	}
}

function sshRemoteAuthority(value: string): string | null {
	const remote = value.trim();
	if (!remote) return null;
	if (remote.startsWith("ssh://")) {
		try {
			const url = new URL(remote);
			const host = url.hostname.replace(/^\[|\]$/g, "").toLowerCase();
			if (!host) return null;
			if (url.port) return `[${host}]:${url.port}`;
			return host.includes(":") ? `[${host}]` : host;
		} catch {
			return null;
		}
	}
	const separator = remote.indexOf(":");
	if (separator <= 0) return null;
	const destination = remote.slice(0, separator);
	const host = (destination.split("@").at(-1) ?? "").toLowerCase();
	return host || null;
}

function sshTargetAuthority(target: string, port?: number): string {
	const host = (target.trim().split("@").at(-1) ?? "").replace(/^\[|\]$/g, "").toLowerCase();
	if (port) return `[${host}]:${port}`;
	return host.includes(":") ? `[${host}]` : host;
}

const SOURCE_FIELDS = {
	https: {
		label: "Repository source",
		placeholder: "owner/repo or full HTTPS URL",
		help: "Use owner/repo for GitHub, or enter a full HTTPS Git URL.",
	},
	ssh: {
		label: "Repository source",
		placeholder: "git@github.com:example/mcp-tools.git",
		help: undefined,
	},
	local: {
		label: "Server-side local path",
		placeholder: "/srv/mcp-tools",
		help: undefined,
	},
} as const;

interface RepositoryInput {
	sourceKind: "https" | "ssh" | "local";
	sourceValue: string;
	privateHttps: boolean;
	credentialId: string;
	sshTargetId: string;
	alias: string;
	repositoryId: string;
	requestedRef: string;
}

function buildRepositoryRequest(input: RepositoryInput): RepositoryRequest | null {
	const source = input.sourceKind === "https" ? normalizeHttpsSource(input.sourceValue) : input.sourceValue.trim();
	const alias = input.alias.trim() || repositoryName(source);
	if (!(alias && source)) return null;
	const common = {
		...(input.repositoryId.trim() ? { id: input.repositoryId.trim() } : {}),
		alias,
		ref: input.requestedRef.trim() || "HEAD",
		discovery: "explicit" as const,
	};
	switch (input.sourceKind) {
		case "https":
			if (input.privateHttps && !input.credentialId) return null;
			return {
				...common,
				source: { kind: "https", url: source, private: input.privateHttps },
				...(input.privateHttps ? { httpsCredentialId: Number(input.credentialId) } : {}),
			};
		case "ssh":
			if (!input.sshTargetId) return null;
			return {
				...common,
				source: { kind: "ssh", remote: source },
				sshTargetId: Number(input.sshTargetId),
			};
		case "local":
			return { ...common, source: { kind: "local", path: source } };
	}
}

function candidatesToInstall(
	mode: "selected" | "all",
	preview: RepositoryPreview,
	selected: Set<string>,
): ManagedCandidate[] {
	if (mode === "all") return preview.candidates;
	return preview.candidates.filter((candidate) => selected.has(candidate.identity));
}

interface HttpsAccessFieldsProps {
	privateHttps: boolean;
	credentialId: string;
	credentials: GitHttpsCredential[];
	onPrivateChange: (checked: boolean) => void;
	onCredentialChange: (value: string) => void;
}

function HttpsAccessFields({
	privateHttps,
	credentialId,
	credentials,
	onPrivateChange,
	onCredentialChange,
}: HttpsAccessFieldsProps): VNode {
	return (
		<div>
			<CheckboxField label="Private HTTPS repository" checked={privateHttps} onChange={onPrivateChange} />
			{privateHttps && (
				<>
					<SelectField
						label="HTTPS credential"
						value={credentialId}
						onChange={onCredentialChange}
						options={[
							{ value: "", label: "Select credential" },
							...credentials.map((credential) => ({
								value: String(credential.id),
								label: `${credential.username}@${credential.host}`,
							})),
						]}
					/>
					{credentials.length === 0 && (
						<p className="-mt-2 mb-3 text-xs text-[var(--muted)]">
							No credential matches this host.{" "}
							<a href="#mcp-repository-credentials" className="text-[var(--accent)] underline">
								Connect GitHub or add one below.
							</a>
						</p>
					)}
				</>
			)}
		</div>
	);
}

interface SshTargetFieldProps {
	selectedId: string;
	targets: SshTargetMetadata[];
	onChange: (value: string) => void;
}

function SshTargetField({ selectedId, targets, onChange }: SshTargetFieldProps): VNode {
	return (
		<div>
			<SelectField
				label="Managed SSH target"
				value={selectedId}
				onChange={onChange}
				options={[
					{ value: "", label: "Select managed target" },
					...targets.map((target) => ({
						value: String(target.id),
						label: `${target.label} (${target.target}${target.port ? `:${target.port}` : ""})`,
					})),
				]}
				help="Only managed-key targets with a strict host pin and the same host and port are eligible."
			/>
			{targets.length === 0 && (
				<p className="-mt-2 mb-3 text-xs text-[var(--muted)]">
					Enter the SSH remote, then{" "}
					<a href="/settings/ssh" className="text-[var(--accent)] underline">
						configure a matching key and pinned target in SSH settings.
					</a>
				</p>
			)}
		</div>
	);
}

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
	const advanced = useSignal(false);
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
		return buildRepositoryRequest({
			sourceKind: sourceKind.value,
			sourceValue: sourceValue.value,
			privateHttps: privateHttps.value,
			credentialId: credentialId.value,
			sshTargetId: sshTargetId.value,
			alias: alias.value,
			repositoryId: repositoryId.value,
			requestedRef: requestedRef.value,
		});
	}

	async function loadPreview(): Promise<void> {
		const repository = request();
		if (!repository) {
			onMessage("Complete the repository source and required credential fields before previewing.", true);
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
		const candidates = candidatesToInstall(mode, currentPreview, selected.value);
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

	const sourceField = SOURCE_FIELDS[sourceKind.value];
	const currentHttpsAuthority = httpsAuthority(sourceValue.value);
	const matchingHttpsCredentials = currentHttpsAuthority
		? credentials.credentials.filter((credential) => credential.host.toLowerCase() === currentHttpsAuthority)
		: [];
	const currentSshAuthority = sshRemoteAuthority(sourceValue.value);
	const matchingSshTargets = currentSshAuthority
		? credentials.sshTargets.filter(
				(target) =>
					target.authMode === "managed" &&
					typeof target.keyId === "number" &&
					target.hasKnownHost &&
					sshTargetAuthority(target.target, target.port) === currentSshAuthority,
			)
		: [];
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
			<div className="max-w-[720px]">
				<TextField
					label={sourceField.label}
					value={sourceValue.value}
					onInput={(value) => {
						sourceValue.value = value;
						credentialId.value = "";
						sshTargetId.value = "";
						clearPreview();
					}}
					placeholder={sourceField.placeholder}
					help={sourceField.help}
					monospace
				/>
				{sourceKind.value === "https" && (
					<HttpsAccessFields
						privateHttps={privateHttps.value}
						credentialId={credentialId.value}
						credentials={matchingHttpsCredentials}
						onPrivateChange={(checked) => {
							privateHttps.value = checked;
							credentialId.value = "";
							clearPreview();
						}}
						onCredentialChange={(value) => {
							credentialId.value = value;
							clearPreview();
						}}
					/>
				)}
				{sourceKind.value === "ssh" && (
					<SshTargetField
						selectedId={sshTargetId.value}
						targets={matchingSshTargets}
						onChange={(value) => {
							sshTargetId.value = value;
							clearPreview();
						}}
					/>
				)}
			</div>
			<button
				type="button"
				className="mb-3 text-xs text-[var(--accent)] underline"
				aria-expanded={advanced.value}
				onClick={() => (advanced.value = !advanced.value)}
			>
				{advanced.value ? "Hide advanced options" : "Advanced options"}
			</button>
			{advanced.value && (
				<div className="mb-3 grid gap-x-4 rounded-lg border border-[var(--border)] bg-[var(--surface)] p-3 sm:grid-cols-2">
					<SelectField
						label="Source type"
						value={sourceKind.value}
						onChange={changeSourceKind}
						options={[
							{ value: "https", label: "GitHub or HTTPS" },
							{ value: "ssh", label: "SSH" },
							{ value: "local", label: "Server-side local path" },
						]}
					/>
					<TextField
						label="Alias (optional)"
						value={alias.value}
						onInput={(value) => {
							alias.value = value;
							clearPreview();
						}}
						placeholder={repositoryName(sourceValue.value) || "company-tools"}
						help="Defaults to the repository name."
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
				</div>
			)}
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
