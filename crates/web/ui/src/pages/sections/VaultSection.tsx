// ── Vault (Encryption) section ──────────────────────────────

import type { VNode } from "preact";
import { useEffect, useState } from "preact/hooks";
import { SectionHeading, StatusMessage } from "../../components/forms";
import * as gon from "../../gon";
import { refresh as refreshGon } from "../../gon";
import { targetValue } from "../../typed-events";
import type { VaultStatus } from "../../types/gon";
import { rerender } from "./_shared";

interface DisabledVaultStateProps {
	error: string | null;
	success: string | null;
}

function DisabledVaultState({ error, success }: DisabledVaultStateProps): VNode {
	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<SectionHeading title="Encryption" />
			<p className="text-xs text-[var(--muted)]">Encryption at rest is disabled or not available in this build.</p>
			<StatusMessage error={error} success={success} />
		</div>
	);
}

interface DisableVaultFormProps {
	vaultStatus: string;
	onDisabled: (message: string) => void;
	onError: (message: string) => void;
}

function DisableVaultForm({ vaultStatus, onDisabled, onError }: DisableVaultFormProps): VNode {
	const [disablePw, setDisablePw] = useState("");
	const [disabling, setDisabling] = useState(false);
	const needsPassword = vaultStatus === "sealed";

	function onDisableVault(e: Event): void {
		e.preventDefault();
		setDisabling(true);
		rerender();
		fetch("/api/auth/vault/disable", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ password: disablePw || undefined }),
		})
			.then((r) => {
				if (r.ok) {
					setDisablePw("");
					onDisabled(
						"Vault disabled. Stored secrets were decrypted; restart Moltis to fully remove vault startup behavior.",
					);
					return;
				}
				return r.text().then((t) => onError(t || "Disable failed"));
			})
			.catch((error: Error) => onError(error.message))
			.finally(() => {
				setDisabling(false);
				rerender();
			});
	}

	return (
		<form onSubmit={onDisableVault} className="mt-6 rounded border border-red-900/50 bg-red-950/20 p-3">
			<div className="text-sm font-semibold text-red-200">Disable encryption at rest</div>
			<p className="my-2 text-xs leading-relaxed text-[var(--muted)]">
				This decrypts stored provider, channel, webhook, environment, and SSH secrets so password login no longer
				requires unlocking the vault after restart.
			</p>
			{needsPassword ? (
				<input
					type="password"
					className="provider-key-input mb-2 w-full"
					value={disablePw}
					onInput={(event: Event) => setDisablePw(targetValue(event))}
					placeholder="Vault password required while locked"
				/>
			) : null}
			<button
				type="submit"
				className="provider-btn provider-btn-danger"
				disabled={disabling || (needsPassword && !disablePw.trim())}
			>
				{disabling ? "Disabling..." : "Decrypt secrets and disable vault"}
			</button>
		</form>
	);
}

interface UnlockFormsProps {
	unlockPw: string;
	recoveryKey: string;
	unlockingPw: boolean;
	unlockingRk: boolean;
	onUnlockPw: (event: Event) => void;
	onUnlockRecovery: (event: Event) => void;
	onUnlockPwInput: (value: string) => void;
	onRecoveryKeyInput: (value: string) => void;
}

function UnlockForms({
	unlockPw,
	recoveryKey,
	unlockingPw,
	unlockingRk,
	onUnlockPw,
	onUnlockRecovery,
	onUnlockPwInput,
	onRecoveryKeyInput,
}: UnlockFormsProps): VNode {
	return (
		<div style={{ display: "flex", flexDirection: "column", gap: "12px" }}>
			<form onSubmit={onUnlockPw} style={{ display: "flex", flexDirection: "column", gap: "6px" }}>
				<div className="text-xs text-[var(--muted)]">Unlock with password</div>
				<div style={{ display: "flex", gap: "8px", alignItems: "center" }}>
					<input
						type="password"
						className="provider-key-input"
						style={{ flex: 1 }}
						value={unlockPw}
						onInput={(e: Event) => onUnlockPwInput(targetValue(e))}
						placeholder="Your password"
					/>
					<button type="submit" className="provider-btn" disabled={unlockingPw || !unlockPw.trim()}>
						{unlockingPw ? "Unlocking..." : "Unlock"}
					</button>
				</div>
			</form>
			<div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
				<div style={{ flex: 1, borderTop: "1px solid var(--border)" }} />
				<span className="text-xs text-[var(--muted)]">or</span>
				<div style={{ flex: 1, borderTop: "1px solid var(--border)" }} />
			</div>
			<form onSubmit={onUnlockRecovery} style={{ display: "flex", flexDirection: "column", gap: "6px" }}>
				<div className="text-xs text-[var(--muted)]">Unlock with recovery key</div>
				<div style={{ display: "flex", gap: "8px", alignItems: "center" }}>
					<input
						type="password"
						className="provider-key-input"
						style={{ flex: 1, fontFamily: "var(--font-mono)", fontSize: ".78rem" }}
						value={recoveryKey}
						onInput={(e: Event) => onRecoveryKeyInput(targetValue(e))}
						placeholder="XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX"
					/>
					<button type="submit" className="provider-btn" disabled={unlockingRk || !recoveryKey.trim()}>
						{unlockingRk ? "Unlocking..." : "Unlock"}
					</button>
				</div>
			</form>
		</div>
	);
}

function VaultIntro(): VNode {
	return (
		<div className="rounded border border-[var(--border)] bg-[var(--surface2)] p-3 mb-4">
			<p className="text-xs text-[var(--muted)] leading-relaxed m-0 mb-1.5">
				Your API keys and secrets are encrypted at rest using{" "}
				<strong className="text-[var(--text)]">XChaCha20-Poly1305</strong> AEAD with keys derived from your password via{" "}
				<strong className="text-[var(--text)]">Argon2id</strong>.
			</p>
			<p className="text-xs text-[var(--muted)] leading-relaxed m-0 mb-1.5">
				The vault uses a two-layer key hierarchy: your password derives a Key Encryption Key (KEK) which unwraps a
				random 256-bit Data Encryption Key (DEK). Changing your password only re-wraps the DEK; all encrypted data stays
				intact. A recovery key (shown once at setup) provides emergency access if you forget your password.
			</p>
			<p className="text-xs text-[var(--muted)] leading-relaxed m-0">
				The vault locks automatically when the server restarts and unlocks when you log in.
			</p>
		</div>
	);
}

interface VaultStatusRowProps {
	vaultStatus: VaultStatus;
	hasPassword: boolean;
}

function VaultStatusRow({ vaultStatus, hasPassword }: VaultStatusRowProps): VNode {
	const label = vaultStatus === "unsealed" ? "Unlocked" : vaultStatus === "sealed" ? "Locked" : "Off";
	const detail =
		vaultStatus === "unsealed"
			? "Your API keys and secrets are encrypted in the database. Everything is working."
			: vaultStatus === "sealed"
				? "Log in or unlock below to access your encrypted keys."
				: hasPassword
					? "Password authentication is configured, but the vault has not been initialized yet."
					: "Set a password in Authentication settings to start encrypting your stored keys.";
	return (
		<div style={{ display: "flex", alignItems: "center", gap: "8px", marginBottom: "12px" }}>
			<span
				className={`provider-item-badge ${vaultStatus === "unsealed" ? "configured" : vaultStatus === "sealed" ? "warning" : "muted"}`}
			>
				{label}
			</span>
			<span className="text-xs text-[var(--muted)]">{detail}</span>
		</div>
	);
}

interface InitializeVaultFormProps {
	password: string;
	initializing: boolean;
	recoveryKey: string | null;
	onPasswordInput: (value: string) => void;
	onInitialize: (event: Event) => void;
}

function InitializeVaultForm({
	password,
	initializing,
	recoveryKey,
	onPasswordInput,
	onInitialize,
}: InitializeVaultFormProps): VNode {
	return (
		<div className="mt-3 rounded border border-[var(--border)] bg-[var(--surface2)] p-3">
			{recoveryKey ? (
				<>
					<div className="text-sm font-semibold text-[var(--text)]">Vault initialized. Save this recovery key.</div>
					<p className="my-2 text-xs leading-relaxed text-[var(--muted)]">
						This key is shown once. Store it somewhere safe so you can unlock the vault if you forget your password.
					</p>
					<code className="block select-all rounded border border-[var(--border)] bg-[var(--bg)] p-2 font-mono text-xs">
						{recoveryKey}
					</code>
				</>
			) : (
				<form onSubmit={onInitialize}>
					<div className="text-sm font-semibold text-[var(--text)]">Initialize encryption vault</div>
					<p className="my-2 text-xs leading-relaxed text-[var(--muted)]">
						Use your current password to create the encrypted vault and receive a one-time recovery key.
					</p>
					<div className="flex items-center gap-2">
						<input
							type="password"
							className="provider-key-input flex-1"
							value={password}
							onInput={(event: Event) => onPasswordInput(targetValue(event))}
							placeholder="Current password"
						/>
						<button type="submit" className="provider-btn" disabled={initializing || !password.trim()}>
							{initializing ? "Initializing..." : "Initialize vault"}
						</button>
					</div>
				</form>
			)}
		</div>
	);
}

export function VaultSection(): VNode {
	const [vaultStatus, setVaultStatus] = useState<VaultStatus | null>(gon.get("vault_status") ?? null);
	const [hasPassword, setHasPassword] = useState(gon.get("auth_has_password") === true);
	const [unlockPw, setUnlockPw] = useState("");
	const [recoveryKey, setRecoveryKey] = useState("");
	const [initializePw, setInitializePw] = useState("");
	const [initializeRecoveryKey, setInitializeRecoveryKey] = useState<string | null>(null);
	const [msg, setMsg] = useState<string | null>(null);
	const [err, setErr] = useState<string | null>(null);
	const [unlockingPw, setUnlockingPw] = useState(false);
	const [unlockingRk, setUnlockingRk] = useState(false);
	const [initializing, setInitializing] = useState(false);

	useEffect(() => {
		const onVaultStatusChange = (val: VaultStatus | undefined): void => {
			setVaultStatus(val ?? null);
			rerender();
		};
		const onAuthHasPasswordChange = (val: boolean): void => {
			setHasPassword(val === true);
			rerender();
		};
		gon.onChange("vault_status", onVaultStatusChange);
		gon.onChange("auth_has_password", onAuthHasPasswordChange);
		return () => {
			gon.offChange("vault_status", onVaultStatusChange);
			gon.offChange("auth_has_password", onAuthHasPasswordChange);
		};
	}, []);

	function onInitializeVault(e: Event): void {
		e.preventDefault();
		if (!initializePw.trim()) return;
		setErr(null);
		setMsg(null);
		setInitializing(true);
		rerender();
		fetch("/api/auth/vault/initialize", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ password: initializePw }),
		})
			.then((r) => {
				if (!r.ok) return r.text().then((t) => setErr(t || "Vault initialization failed"));
				return r.json().then((data: { recovery_key?: string; status?: VaultStatus }) => {
					setInitializePw("");
					setInitializeRecoveryKey(data.recovery_key || null);
					setVaultStatus(data.status || "sealed");
					setMsg("Vault initialized.");
					refreshGon();
				});
			})
			.catch((error: Error) => setErr(error.message))
			.finally(() => {
				setInitializing(false);
				rerender();
			});
	}

	function onUnlockPw(e: Event): void {
		e.preventDefault();
		if (!unlockPw.trim()) return;
		setErr(null);
		setMsg(null);
		setUnlockingPw(true);
		rerender();
		doUnlock("/api/auth/vault/unlock", { password: unlockPw }, () => setUnlockingPw(false));
	}

	function onUnlockRecovery(e: Event): void {
		e.preventDefault();
		if (!recoveryKey.trim()) return;
		setErr(null);
		setMsg(null);
		setUnlockingRk(true);
		rerender();
		doUnlock("/api/auth/vault/recovery", { recovery_key: recoveryKey }, () => setUnlockingRk(false));
	}

	function doUnlock(url: string, body: Record<string, string>, done: () => void): void {
		fetch(url, {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify(body),
		})
			.then((r) => {
				if (r.ok) {
					setMsg("Vault unlocked.");
					setUnlockPw("");
					setRecoveryKey("");
					refreshGon();
				} else {
					return r.text().then((t) => setErr(t || "Unlock failed"));
				}
				done();
				rerender();
			})
			.catch((error: Error) => {
				setErr(error.message);
				done();
				rerender();
			});
	}

	if (!vaultStatus || vaultStatus === "disabled") {
		return <DisabledVaultState error={err} success={msg} />;
	}

	function onVaultDisabled(message: string): void {
		setErr(null);
		setMsg(message);
		setVaultStatus("disabled");
	}

	function onVaultDisableError(message: string): void {
		setMsg(null);
		setErr(message);
	}

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<SectionHeading title="Encryption" />

			<div style={{ maxWidth: "600px" }}>
				<VaultIntro />
				<VaultStatusRow vaultStatus={vaultStatus} hasPassword={hasPassword} />

				{vaultStatus === "sealed" ? (
					<UnlockForms
						unlockPw={unlockPw}
						recoveryKey={recoveryKey}
						unlockingPw={unlockingPw}
						unlockingRk={unlockingRk}
						onUnlockPw={onUnlockPw}
						onUnlockRecovery={onUnlockRecovery}
						onUnlockPwInput={setUnlockPw}
						onRecoveryKeyInput={setRecoveryKey}
					/>
				) : null}

				{initializeRecoveryKey || (vaultStatus === "uninitialized" && hasPassword) ? (
					<InitializeVaultForm
						password={initializePw}
						initializing={initializing}
						recoveryKey={initializeRecoveryKey}
						onPasswordInput={setInitializePw}
						onInitialize={onInitializeVault}
					/>
				) : null}

				{vaultStatus === "uninitialized" && !hasPassword ? (
					<div style={{ marginTop: "4px" }}>
						<a
							href="/settings/security"
							className="provider-btn provider-btn-secondary"
							style={{ fontSize: ".75rem", textDecoration: "none", display: "inline-block" }}
						>
							Set a password
						</a>
					</div>
				) : null}

				{vaultStatus === "unsealed" || vaultStatus === "sealed" ? (
					<DisableVaultForm vaultStatus={vaultStatus} onDisabled={onVaultDisabled} onError={onVaultDisableError} />
				) : null}

				<StatusMessage error={err} success={msg} />
			</div>
		</div>
	);
}
