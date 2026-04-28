// ── SSH section ──────────────────────────────────────────────

import type { VNode } from "preact";
import { useEffect, useState } from "preact/hooks";
import { Badge, EmptyState, Loading } from "../../components/forms";
import { TabBar } from "../../components/forms/Tabs";
import * as gon from "../../gon";
import { localizedApiErrorMessage } from "../../helpers";
import { targetChecked, targetValue } from "../../typed-events";
import { showToast } from "../../ui";
import { rerender } from "./_shared";

interface SshKeyEntry {
	id: number;
	name: string;
	fingerprint?: string;
	public_key?: string;
	encrypted?: boolean;
	target_count?: number;
}

interface SshTargetEntry {
	id: number;
	label: string;
	target: string;
	port?: number;
	auth_mode?: string;
	key_name?: string;
	known_host?: string;
	is_default?: boolean;
}

interface TestResult {
	reachable?: boolean;
	failure_hint?: string;
}

export function SshSection(): VNode {
	const [loadingSsh, setLoadingSsh] = useState(true);
	const [keys, setKeys] = useState<SshKeyEntry[]>([]);
	const [targets, setTargets] = useState<SshTargetEntry[]>([]);
	const [sshMsg, setSshMsg] = useState<string | null>(null);
	const [sshErr, setSshErr] = useState<string | null>(null);
	const [busyAction, setBusyAction] = useState("");
	const [generateName, setGenerateName] = useState("");
	const [importName, setImportName] = useState("");
	const [importPrivateKey, setImportPrivateKey] = useState("");
	const [importPassphrase, setImportPassphrase] = useState("");
	const [targetLabel, setTargetLabel] = useState("");
	const [targetHost, setTargetHost] = useState("");
	const [targetPort, setTargetPort] = useState("");
	const [targetKnownHost, setTargetKnownHost] = useState("");
	const [targetAuthMode, setTargetAuthMode] = useState("managed");
	const [targetKeyId, setTargetKeyId] = useState("");
	const [targetIsDefault, setTargetIsDefault] = useState(true);
	const [copiedKeyId, setCopiedKeyId] = useState<number | null>(null);
	const [testResults, setTestResults] = useState<Record<number, TestResult>>({});
	const vaultStatus = gon.get("vault_status");

	function setMessage(message: string): void {
		setSshMsg(message);
		setSshErr(null);
	}

	function setError(message: string): void {
		setSshErr(message);
		setSshMsg(null);
	}

	function clearFlash(): void {
		setSshMsg(null);
		setSshErr(null);
	}

	function fetchSshStatus(): Promise<void> {
		setLoadingSsh(true);
		rerender();
		return fetch("/api/ssh")
			.then(async (response) => {
				if (!response.ok) {
					throw new Error(localizedApiErrorMessage(await response.json(), "Failed to load SSH settings"));
				}
				return response.json();
			})
			.then((data: { keys?: SshKeyEntry[]; targets?: SshTargetEntry[] }) => {
				setKeys(data.keys || []);
				setTargets(data.targets || []);
				if (!targetKeyId && (data.keys || []).length > 0) {
					setTargetKeyId(String(data.keys?.[0].id));
				}
				setLoadingSsh(false);
				rerender();
			})
			.catch((error: Error) => {
				setLoadingSsh(false);
				setError(error.message);
				rerender();
			});
	}

	useEffect(() => {
		fetchSshStatus();
	}, []);

	function runSshAction(
		actionKey: string,
		url: string,
		payload: Record<string, unknown> | null,
		successMessage: string,
		afterSuccess?: (data: unknown) => void | Promise<void>,
	): Promise<void> {
		clearFlash();
		setBusyAction(actionKey);
		rerender();
		return fetch(url, {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: payload ? JSON.stringify(payload) : "{}",
		})
			.then(async (response) => {
				if (!response.ok) {
					throw new Error(localizedApiErrorMessage(await response.json(), "SSH action failed"));
				}
				return response.json().catch(() => ({}));
			})
			.then(async (data: unknown) => {
				if (afterSuccess) await afterSuccess(data);
				setMessage(successMessage);
				await fetchSshStatus();
			})
			.catch((error: Error) => {
				setError(error.message);
			})
			.finally(() => {
				setBusyAction("");
				rerender();
			});
	}

	function onGenerateKey(e: Event): void {
		e.preventDefault();
		const name = generateName.trim();
		if (!name) {
			setError("Key name is required.");
			return;
		}
		runSshAction("generate-key", "/api/ssh/keys/generate", { name }, "Deploy key generated.", () => {
			setGenerateName("");
		});
	}

	function onImportKey(e: Event): void {
		e.preventDefault();
		const name = importName.trim();
		if (!name) {
			setError("Key name is required.");
			return;
		}
		if (!importPrivateKey.trim()) {
			setError("Private key is required.");
			return;
		}
		runSshAction(
			"import-key",
			"/api/ssh/keys/import",
			{
				name,
				private_key: importPrivateKey,
				passphrase: importPassphrase.trim() ? importPassphrase : null,
			},
			"Private key imported.",
			() => {
				setImportName("");
				setImportPrivateKey("");
				setImportPassphrase("");
			},
		);
	}

	function onDeleteKey(id: number): void {
		clearFlash();
		setBusyAction(`delete-key:${id}`);
		rerender();
		fetch(`/api/ssh/keys/${id}`, { method: "DELETE" })
			.then(async (response) => {
				if (!response.ok) {
					throw new Error(localizedApiErrorMessage(await response.json(), "Failed to delete key"));
				}
				setMessage("SSH key deleted.");
				await fetchSshStatus();
			})
			.catch((error: Error) => setError(error.message))
			.finally(() => {
				setBusyAction("");
				rerender();
			});
	}

	function onCreateTarget(e: Event): void {
		e.preventDefault();
		const label = targetLabel.trim();
		const target = targetHost.trim();
		const port = targetPort.trim() ? Number.parseInt(targetPort.trim(), 10) : null;
		const keyId = targetAuthMode === "managed" && targetKeyId ? Number.parseInt(targetKeyId, 10) : null;
		if (!label) {
			setError("Target label is required.");
			return;
		}
		if (!target) {
			setError("SSH target is required.");
			return;
		}
		if (targetAuthMode === "managed" && !keyId) {
			setError("Choose a managed SSH key for this target.");
			return;
		}
		if (Number.isNaN(port)) {
			setError("Port must be a valid number.");
			return;
		}
		runSshAction(
			"create-target",
			"/api/ssh/targets",
			{
				label,
				target,
				port,
				auth_mode: targetAuthMode,
				key_id: keyId,
				known_host: targetKnownHost.trim() ? targetKnownHost : null,
				is_default: targetIsDefault,
			},
			"SSH target saved.",
			() => {
				setTargetLabel("");
				setTargetHost("");
				setTargetPort("");
				setTargetKnownHost("");
				setTargetIsDefault(targets.length === 0);
			},
		);
	}

	function onScanCreateTargetHost(): void {
		const target = targetHost.trim();
		const port = targetPort.trim() ? Number.parseInt(targetPort.trim(), 10) : null;
		if (!target) {
			setError("SSH target is required before scanning.");
			return;
		}
		if (Number.isNaN(port)) {
			setError("Port must be a valid number.");
			return;
		}
		clearFlash();
		setBusyAction("scan-create-target");
		rerender();
		fetch("/api/ssh/host-key/scan", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ target, port }),
		})
			.then(async (response) => {
				if (!response.ok) {
					throw new Error(localizedApiErrorMessage(await response.json(), "Failed to scan host key"));
				}
				return response.json();
			})
			.then((data: { known_host?: string; host?: string; port?: number }) => {
				setTargetKnownHost(data.known_host || "");
				setMessage(`Scanned host key for ${data.host}${data.port ? `:${data.port}` : ""}.`);
				showToast("Host key scanned", "success");
				rerender();
			})
			.catch((error: Error) => {
				setError(error.message);
				showToast(error.message, "error");
			})
			.finally(() => {
				setBusyAction("");
				rerender();
			});
	}

	function onDeleteTarget(id: number): void {
		clearFlash();
		setBusyAction(`delete-target:${id}`);
		rerender();
		fetch(`/api/ssh/targets/${id}`, { method: "DELETE" })
			.then(async (response) => {
				if (!response.ok) {
					throw new Error(localizedApiErrorMessage(await response.json(), "Failed to delete target"));
				}
				setMessage("SSH target deleted.");
				await fetchSshStatus();
			})
			.catch((error: Error) => setError(error.message))
			.finally(() => {
				setBusyAction("");
				rerender();
			});
	}

	function onSetDefaultTarget(id: number): void {
		runSshAction(`default-target:${id}`, `/api/ssh/targets/${id}/default`, null, "Default SSH target updated.");
	}

	function onTestTarget(id: number): void {
		clearFlash();
		setBusyAction(`test-target:${id}`);
		rerender();
		fetch(`/api/ssh/targets/${id}/test`, { method: "POST" })
			.then(async (response) => {
				if (!response.ok) {
					throw new Error(localizedApiErrorMessage(await response.json(), "SSH connectivity test failed"));
				}
				return response.json();
			})
			.then((data: TestResult) => {
				setTestResults({
					...testResults,
					[id]: data,
				});
				setMessage(
					data.reachable ? "SSH connectivity test passed." : data.failure_hint || "SSH connectivity test failed.",
				);
				rerender();
			})
			.catch((error: Error) => setError(error.message))
			.finally(() => {
				setBusyAction("");
				rerender();
			});
	}

	function onScanAndPinTarget(entry: SshTargetEntry): void {
		clearFlash();
		setBusyAction(`pin-target:${entry.id}`);
		rerender();
		fetch("/api/ssh/host-key/scan", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ target: entry.target, port: entry.port ?? null }),
		})
			.then(async (response) => {
				if (!response.ok) {
					throw new Error(localizedApiErrorMessage(await response.json(), "Failed to scan host key"));
				}
				return response.json();
			})
			.then(async (scanData: { known_host?: string; host?: string; port?: number }) => {
				const pinResponse = await fetch(`/api/ssh/targets/${entry.id}/pin`, {
					method: "POST",
					headers: { "Content-Type": "application/json" },
					body: JSON.stringify({ known_host: scanData.known_host }),
				});
				if (!pinResponse.ok) {
					throw new Error(localizedApiErrorMessage(await pinResponse.json(), "Failed to pin host key"));
				}
				setMessage(
					`${entry.known_host ? "Refreshed" : "Pinned"} host key for ${scanData.host}${scanData.port ? `:${scanData.port}` : ""}.`,
				);
				showToast(entry.known_host ? "Host pin refreshed" : "Host pinned", "success");
				await fetchSshStatus();
			})
			.catch((error: Error) => {
				setError(error.message);
				showToast(error.message, "error");
			})
			.finally(() => {
				setBusyAction("");
				rerender();
			});
	}

	function onClearTargetPin(entry: SshTargetEntry): void {
		clearFlash();
		setBusyAction(`clear-pin:${entry.id}`);
		rerender();
		fetch(`/api/ssh/targets/${entry.id}/pin`, { method: "DELETE" })
			.then(async (response) => {
				if (!response.ok) {
					throw new Error(localizedApiErrorMessage(await response.json(), "Failed to clear host pin"));
				}
				setMessage(`Cleared host pin for ${entry.label}.`);
				showToast("Host pin cleared", "success");
				await fetchSshStatus();
			})
			.catch((error: Error) => {
				setError(error.message);
				showToast(error.message, "error");
			})
			.finally(() => {
				setBusyAction("");
				rerender();
			});
	}

	function onCopyPublicKey(entry: SshKeyEntry): void {
		navigator.clipboard
			.writeText(entry.public_key || "")
			.then(() => {
				setCopiedKeyId(entry.id);
				setTimeout(() => {
					setCopiedKeyId(null);
					rerender();
				}, 1500);
				rerender();
			})
			.catch((error: Error) => setError(error.message));
	}

	const [sshTab, setSshTab] = useState("keys");

	const sshTabs = [
		{ id: "keys", label: "Deploy Keys", badge: keys.length || undefined },
		{ id: "targets", label: "Targets", badge: targets.length || undefined },
	];

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<h2 className="text-lg font-medium text-[var(--text-strong)]">SSH</h2>
			<p className="text-xs text-[var(--muted)] leading-relaxed max-w-[760px]" style={{ margin: 0 }}>
				Manage outbound SSH keys and named remote exec targets. Current auth path:
				<strong className="text-[var(--text)]">
					{vaultStatus === "unsealed"
						? " vault-backed managed keys are available"
						: vaultStatus === "sealed"
							? " vault is locked, managed keys cannot be used until unlocked"
							: " system OpenSSH remains available, managed keys stay plaintext until the vault is enabled"}
				</strong>
			</p>

			{sshMsg ? <div className="text-xs text-[var(--accent)]">{sshMsg}</div> : null}
			{sshErr ? <div className="text-xs text-[var(--error)]">{sshErr}</div> : null}

			<TabBar tabs={sshTabs} active={sshTab} onChange={setSshTab} />

			{sshTab === "keys" && (
				<div className="flex flex-col gap-4 max-w-[760px]">
					<p className="text-xs text-[var(--muted)] m-0">
						Generate a new keypair for a host, or import an existing private key. Passphrase-protected imports are
						decrypted once and then stored under Moltis control.
					</p>
					<div className="mb-3 rounded border border-[var(--border)] bg-[var(--surface2)] p-2 text-xs text-[var(--muted)] leading-relaxed">
						Recommended flow: generate one deploy key per remote host, copy the public key below, add it to that
						host&apos;s <code className="text-[var(--text)]">~/.ssh/authorized_keys</code>, then pin the host key with
						<code className="text-[var(--text)]">ssh-keyscan -H host</code> when creating the target.
					</div>
					<form onSubmit={onGenerateKey} className="flex flex-col gap-2 mb-4">
						<label className="text-xs text-[var(--muted)]">Generate deploy key</label>
						<div className="flex gap-2 flex-wrap">
							<input
								className="provider-key-input flex-1 min-w-[180px]"
								type="text"
								value={generateName}
								onInput={(e: Event) => setGenerateName(targetValue(e))}
								placeholder="production-box"
							/>
							<button type="submit" className="provider-btn" disabled={busyAction === "generate-key"}>
								{busyAction === "generate-key" ? "Generating\u2026" : "Generate"}
							</button>
						</div>
					</form>

					<form onSubmit={onImportKey} className="flex flex-col gap-2">
						<label className="text-xs text-[var(--muted)]">Import private key</label>
						<input
							className="provider-key-input"
							type="text"
							value={importName}
							onInput={(e: Event) => setImportName(targetValue(e))}
							placeholder="existing-deploy-key"
						/>
						<textarea
							className="provider-key-input min-h-[140px] font-mono text-xs"
							value={importPrivateKey}
							onInput={(e: Event) => setImportPrivateKey(targetValue(e))}
							placeholder="-----BEGIN OPENSSH PRIVATE KEY-----"
						/>
						<input
							className="provider-key-input"
							type="password"
							value={importPassphrase}
							onInput={(e: Event) => setImportPassphrase(targetValue(e))}
							placeholder="Optional import passphrase"
						/>
						<button type="submit" className="provider-btn self-start" disabled={busyAction === "import-key"}>
							{busyAction === "import-key" ? "Importing\u2026" : "Import Key"}
						</button>
					</form>

					<div className="mt-4 flex flex-col gap-2">
						{loadingSsh ? (
							<Loading message="Loading keys..." />
						) : keys.length === 0 ? (
							<EmptyState message="No managed SSH keys yet." />
						) : (
							keys.map((entry) => (
								<div className="provider-item items-start gap-4" key={entry.id}>
									<div className="flex-1 min-w-0">
										<div className="provider-item-name">{entry.name}</div>
										<div className="text-xs text-[var(--muted)] break-all mt-1">
											<span className="text-[var(--text)]">Fingerprint (SHA256):</span> {entry.fingerprint}
										</div>
										<div className="text-xs text-[var(--muted)] mt-1">
											{entry.encrypted ? "Encrypted in vault" : "Stored plaintext until the vault is available"}
											{(entry.target_count ?? 0) > 0
												? `, used by ${entry.target_count} target${entry.target_count === 1 ? "" : "s"}`
												: ""}
										</div>
										<pre className="mt-3 whitespace-pre-wrap break-all rounded border border-[var(--border)] bg-[var(--surface2)] p-2 text-[11px] leading-relaxed text-[var(--muted)]">
											{entry.public_key}
										</pre>
									</div>
									<div className="flex flex-col gap-2 shrink-0 self-start">
										<button
											type="button"
											className="provider-btn provider-btn-secondary"
											onClick={() => onCopyPublicKey(entry)}
										>
											{copiedKeyId === entry.id ? "Copied" : "Copy Public Key"}
										</button>
										<button
											type="button"
											className="provider-btn provider-btn-danger"
											onClick={() => onDeleteKey(entry.id)}
											disabled={busyAction === `delete-key:${entry.id}` || (entry.target_count ?? 0) > 0}
										>
											{busyAction === `delete-key:${entry.id}` ? "Deleting\u2026" : "Delete"}
										</button>
									</div>
								</div>
							))
						)}
					</div>
				</div>
			)}

			{sshTab === "targets" && (
				<div className="flex flex-col gap-4 max-w-[760px]">
					<p className="text-xs text-[var(--muted)] m-0">
						Add named hosts for remote execution. Targets can use your system OpenSSH setup or one of the managed keys.
					</p>
					<form onSubmit={onCreateTarget} className="flex flex-col gap-2 mb-4">
						<input
							className="provider-key-input"
							type="text"
							value={targetLabel}
							onInput={(e: Event) => setTargetLabel(targetValue(e))}
							placeholder="prod-box"
						/>
						<input
							className="provider-key-input"
							type="text"
							value={targetHost}
							onInput={(e: Event) => setTargetHost(targetValue(e))}
							placeholder="deploy@example.com"
						/>
						<div className="flex gap-2 flex-wrap">
							<input
								className="provider-key-input w-[120px]"
								type="number"
								min={1}
								max={65535}
								value={targetPort}
								onInput={(e: Event) => setTargetPort(targetValue(e))}
								placeholder="22"
							/>
							<select
								className="provider-key-input flex-1 min-w-[180px]"
								value={targetAuthMode}
								onInput={(e: Event) => setTargetAuthMode(targetValue(e))}
							>
								<option value="managed">Managed key</option>
								<option value="system">System OpenSSH</option>
							</select>
						</div>
						<textarea
							className="provider-key-input min-h-[96px] font-mono text-xs"
							value={targetKnownHost}
							onInput={(e: Event) => setTargetKnownHost(targetValue(e))}
							placeholder="Optional known_hosts line from ssh-keyscan -H host"
						/>
						<div className="text-xs text-[var(--muted)]">
							If you paste a <code className="text-[var(--text)]">known_hosts</code> line here, Moltis will use strict
							host-key checking for this target instead of trusting your global SSH config.
						</div>
						<button
							type="button"
							className="provider-btn provider-btn-secondary self-start"
							onClick={onScanCreateTargetHost}
							disabled={busyAction === "scan-create-target"}
						>
							{busyAction === "scan-create-target" ? "Scanning\u2026" : "Scan Host Key"}
						</button>
						{targetAuthMode === "managed" ? (
							<select
								className="provider-key-input"
								value={targetKeyId}
								onInput={(e: Event) => setTargetKeyId(targetValue(e))}
							>
								<option value="">Choose a managed key</option>
								{keys.map((entry) => (
									<option key={entry.id} value={entry.id}>
										{entry.name}
									</option>
								))}
							</select>
						) : null}
						{targetAuthMode === "managed" && keys.length === 0 ? (
							<div className="text-xs text-[var(--muted)]">
								Generate or import a deploy key first. Moltis cannot connect with a managed target until a private key
								exists.
							</div>
						) : null}
						<label className="text-xs text-[var(--muted)] flex items-center gap-2">
							<input
								type="checkbox"
								checked={targetIsDefault}
								onInput={(e: Event) => setTargetIsDefault(targetChecked(e))}
							/>
							Set as default remote SSH target
						</label>
						<button
							type="submit"
							className="provider-btn self-start"
							disabled={busyAction === "create-target" || (targetAuthMode === "managed" && keys.length === 0)}
						>
							{busyAction === "create-target" ? "Saving\u2026" : "Add Target"}
						</button>
					</form>

					<div className="flex flex-col gap-2">
						{loadingSsh ? (
							<Loading message="Loading targets..." />
						) : targets.length === 0 ? (
							<EmptyState message="No SSH targets configured." />
						) : (
							targets.map((entry) => (
								<div className="provider-item" key={entry.id}>
									<div className="flex-1 min-w-0">
										<div className="provider-item-name flex items-center gap-2 flex-wrap">
											<span>{entry.label}</span>
											{entry.is_default ? <Badge label="Default" variant="configured" /> : null}
											<Badge label={entry.auth_mode === "managed" ? "Managed key" : "System SSH"} />
											{entry.known_host ? (
												<Badge label="Host pinned" variant="configured" />
											) : (
												<Badge label="Uses global known_hosts" variant="warning" />
											)}
										</div>
										<div className="text-xs text-[var(--muted)] break-all">
											{entry.target}
											{entry.port ? `:${entry.port}` : ""}
										</div>
										<div className="text-xs text-[var(--muted)]">
											{entry.key_name ? `Key: ${entry.key_name}` : "Uses your local ssh config / agent"}
										</div>
										{testResults[entry.id] ? (
											<div className="mt-1">
												<div
													className={`text-xs ${testResults[entry.id].reachable ? "text-[var(--accent)]" : "text-[var(--error)]"}`}
												>
													{testResults[entry.id].reachable ? "Reachable" : "Unreachable"}
												</div>
												{testResults[entry.id].failure_hint ? (
													<div className="text-xs text-[var(--text-muted)] mt-1">
														Hint: {testResults[entry.id].failure_hint}
													</div>
												) : null}
											</div>
										) : null}
									</div>
									<div className="flex flex-col gap-2">
										<button
											type="button"
											className="provider-btn provider-btn-secondary"
											onClick={() => onTestTarget(entry.id)}
											disabled={busyAction === `test-target:${entry.id}`}
										>
											{busyAction === `test-target:${entry.id}` ? "Testing\u2026" : "Test"}
										</button>
										<button
											type="button"
											className="provider-btn provider-btn-secondary"
											onClick={() => onScanAndPinTarget(entry)}
											disabled={busyAction === `pin-target:${entry.id}`}
										>
											{busyAction === `pin-target:${entry.id}`
												? "Scanning\u2026"
												: entry.known_host
													? "Refresh Pin"
													: "Scan & Pin"}
										</button>
										{entry.known_host ? (
											<button
												type="button"
												className="provider-btn provider-btn-secondary"
												onClick={() => onClearTargetPin(entry)}
												disabled={busyAction === `clear-pin:${entry.id}`}
											>
												{busyAction === `clear-pin:${entry.id}` ? "Clearing\u2026" : "Clear Pin"}
											</button>
										) : null}
										{entry.is_default ? null : (
											<button
												type="button"
												className="provider-btn provider-btn-secondary"
												onClick={() => onSetDefaultTarget(entry.id)}
												disabled={busyAction === `default-target:${entry.id}`}
											>
												Make Default
											</button>
										)}
										<button
											type="button"
											className="provider-btn provider-btn-danger"
											onClick={() => onDeleteTarget(entry.id)}
											disabled={busyAction === `delete-target:${entry.id}`}
										>
											{busyAction === `delete-target:${entry.id}` ? "Deleting\u2026" : "Delete"}
										</button>
									</div>
								</div>
							))
						)}
					</div>
				</div>
			)}
		</div>
	);
}
