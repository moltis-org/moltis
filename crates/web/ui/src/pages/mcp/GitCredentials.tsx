import { useSignal } from "@preact/signals";
import type { VNode } from "preact";
import { TextField } from "../../components/forms";
import { requestConfirm } from "../../ui";
import { mcpRepositoryRpc } from "./rpc";
import type { CredentialMutationResponse, GitCredentialsResponse, GitHttpsCredential } from "./types";

interface GitCredentialsProps {
	data: GitCredentialsResponse;
	onChanged: () => Promise<void>;
	onMessage: (message: string, error?: boolean) => void;
}

export function GitCredentials({ data, onChanged, onMessage }: GitCredentialsProps): VNode {
	const editing = useSignal<GitHttpsCredential | null>(null);
	const host = useSignal("");
	const username = useSignal("");
	const token = useSignal("");
	const busy = useSignal(false);

	function reset(): void {
		editing.value = null;
		host.value = "";
		username.value = "";
		token.value = "";
	}

	function startEdit(credential: GitHttpsCredential): void {
		editing.value = credential;
		host.value = credential.host;
		username.value = credential.username;
		token.value = "";
	}

	async function save(): Promise<void> {
		if (!(host.value.trim() && username.value.trim() && token.value)) return;
		busy.value = true;
		try {
			const method = editing.value ? "mcp.git.credentials.update" : "mcp.git.credentials.create";
			const result = await mcpRepositoryRpc<CredentialMutationResponse>(method, {
				...(editing.value ? { id: editing.value.id } : {}),
				host: host.value.trim(),
				username: username.value.trim(),
				token: token.value,
			});
			token.value = "";
			onMessage(
				result.storageWarning || "HTTPS credential saved. The token will not be displayed again.",
				Boolean(result.storageWarning),
			);
			reset();
			await onChanged();
		} catch (error) {
			token.value = "";
			onMessage(error instanceof Error ? error.message : "Credential save failed", true);
		} finally {
			busy.value = false;
		}
	}

	async function remove(credential: GitHttpsCredential): Promise<void> {
		const confirmed = await requestConfirm(`Remove HTTPS credential for ${credential.username}@${credential.host}?`, {
			confirmLabel: "Remove",
			danger: true,
		});
		if (!confirmed) return;
		busy.value = true;
		try {
			await mcpRepositoryRpc("mcp.git.credentials.remove", { id: credential.id });
			onMessage("HTTPS credential removed.");
			if (editing.value?.id === credential.id) reset();
			await onChanged();
		} catch (error) {
			onMessage(error instanceof Error ? error.message : "Credential removal failed", true);
		} finally {
			busy.value = false;
		}
	}

	return (
		<section className="rounded-xl border border-[var(--border)] bg-[var(--surface2)] p-4 sm:p-5">
			<div className="mb-4">
				<h3 className="text-sm font-medium text-[var(--text-strong)]">Repository credentials</h3>
				<p className="mt-1 text-xs text-[var(--muted)]">
					Tokens are submitted once and never returned. SSH entries expose metadata and pin availability only.
				</p>
			</div>
			<div className="grid gap-4 lg:grid-cols-2">
				<div>
					<h4 className="mb-2 text-xs font-medium uppercase tracking-wide text-[var(--muted)]">HTTPS credentials</h4>
					<div className="grid gap-2">
						{data.credentials.length === 0 && <p className="text-xs text-[var(--muted)]">No HTTPS credentials.</p>}
						{data.credentials.map((credential) => (
							<div
								key={credential.id}
								className="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-[var(--border)] bg-[var(--surface)] p-3"
							>
								<div>
									<div className="text-sm text-[var(--text-strong)]">
										{credential.username}@{credential.host}
									</div>
									<div className={`text-xs ${credential.encrypted ? "text-[var(--ok)]" : "text-[var(--warn)]"}`}>
										{credential.encrypted
											? "Encrypted storage"
											: "Plaintext storage: vault encryption unavailable or sealed"}
									</div>
								</div>
								<div className="flex gap-1.5">
									<button
										type="button"
										className="provider-btn provider-btn-secondary provider-btn-sm"
										onClick={() => startEdit(credential)}
									>
										Replace token
									</button>
									<button
										type="button"
										className="provider-btn provider-btn-danger provider-btn-sm"
										onClick={() => remove(credential)}
									>
										Remove credential
									</button>
								</div>
							</div>
						))}
					</div>
					<div className="mt-4 rounded-lg border border-[var(--border)] bg-[var(--surface)] p-3">
						<h5 className="mb-3 text-sm font-medium text-[var(--text-strong)]">
							{editing.value ? "Replace HTTPS credential" : "Add HTTPS credential"}
						</h5>
						<TextField
							label="Git host"
							value={host.value}
							onInput={(value) => (host.value = value)}
							placeholder="github.com"
						/>
						<TextField label="Username" value={username.value} onInput={(value) => (username.value = value)} />
						<TextField
							label="Access token"
							type="password"
							value={token.value}
							onInput={(value) => (token.value = value)}
							autoComplete="new-password"
							help="Required for create and update. Sent only when you save."
						/>
						<div className="flex gap-2">
							<button
								type="button"
								className="provider-btn"
								onClick={save}
								disabled={busy.value || !(host.value.trim() && username.value.trim() && token.value)}
							>
								{editing.value ? "Update credential" : "Create credential"}
							</button>
							{editing.value && (
								<button type="button" className="provider-btn provider-btn-secondary" onClick={reset}>
									Cancel
								</button>
							)}
						</div>
					</div>
				</div>
				<div>
					<h4 className="mb-2 text-xs font-medium uppercase tracking-wide text-[var(--muted)]">Managed SSH targets</h4>
					<div className="grid gap-2">
						{data.sshTargets.length === 0 && <p className="text-xs text-[var(--muted)]">No SSH targets configured.</p>}
						{data.sshTargets.map((target) => (
							<div key={target.id} className="rounded-lg border border-[var(--border)] bg-[var(--surface)] p-3">
								<div className="flex flex-wrap items-center gap-2">
									<span className="text-sm font-medium text-[var(--text-strong)]">{target.label}</span>
									<span className="rounded-full bg-[var(--surface2)] px-2 py-0.5 text-[0.65rem] text-[var(--muted)]">
										{target.authMode}
									</span>
									<span className={`text-xs ${target.hasKnownHost ? "text-[var(--ok)]" : "text-[var(--warn)]"}`}>
										{target.hasKnownHost ? "Host pin available" : "Host pin missing"}
									</span>
								</div>
								<div className="mt-1 font-mono text-xs text-[var(--muted)]">
									{target.target}
									{target.port ? `:${target.port}` : ""} / key {target.keyName || "not assigned"}
								</div>
							</div>
						))}
					</div>
				</div>
			</div>
		</section>
	);
}
