// ── Vault (Encryption) section ──────────────────────────────

import type { VNode } from "preact";
import { useEffect, useState } from "preact/hooks";
import { SectionHeading, StatusMessage } from "../../components/forms";
import * as gon from "../../gon";
import { refresh as refreshGon } from "../../gon";
import { targetValue } from "../../typed-events";
import { rerender } from "./_shared";

export function VaultSection(): VNode {
	const [vaultStatus, setVaultStatus] = useState(gon.get("vault_status") || null);
	const [unlockPw, setUnlockPw] = useState("");
	const [recoveryKey, setRecoveryKey] = useState("");
	const [msg, setMsg] = useState<string | null>(null);
	const [err, setErr] = useState<string | null>(null);
	const [unlockingPw, setUnlockingPw] = useState(false);
	const [unlockingRk, setUnlockingRk] = useState(false);

	useEffect(() => {
		return gon.onChange("vault_status", (val: unknown) => {
			setVaultStatus(val as string);
			rerender();
		});
	}, []);

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
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<SectionHeading title="Encryption" />
				<p className="text-xs text-[var(--muted)]">Encryption at rest is not available in this build.</p>
			</div>
		);
	}

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<SectionHeading title="Encryption" />

			<div style={{ maxWidth: "600px" }}>
				<div className="rounded border border-[var(--border)] bg-[var(--surface2)] p-3 mb-4">
					<p className="text-xs text-[var(--muted)] leading-relaxed m-0 mb-1.5">
						Your API keys and secrets are encrypted at rest using{" "}
						<strong className="text-[var(--text)]">XChaCha20-Poly1305</strong> AEAD with keys derived from your password
						via <strong className="text-[var(--text)]">Argon2id</strong>.
					</p>
					<p className="text-xs text-[var(--muted)] leading-relaxed m-0 mb-1.5">
						The vault uses a two-layer key hierarchy: your password derives a Key Encryption Key (KEK) which unwraps a
						random 256-bit Data Encryption Key (DEK). Changing your password only re-wraps the DEK {"\u2014"} all
						encrypted data stays intact. A recovery key (shown once at setup) provides emergency access if you forget
						your password.
					</p>
					<p className="text-xs text-[var(--muted)] leading-relaxed m-0">
						The vault locks automatically when the server restarts and unlocks when you log in.
					</p>
				</div>

				<div style={{ display: "flex", alignItems: "center", gap: "8px", marginBottom: "12px" }}>
					<span
						className={`provider-item-badge ${vaultStatus === "unsealed" ? "configured" : vaultStatus === "sealed" ? "warning" : "muted"}`}
					>
						{vaultStatus === "unsealed" ? "Unlocked" : vaultStatus === "sealed" ? "Locked" : "Off"}
					</span>
					<span className="text-xs text-[var(--muted)]">
						{vaultStatus === "unsealed"
							? "Your API keys and secrets are encrypted in the database. Everything is working."
							: vaultStatus === "sealed"
								? "Log in or unlock below to access your encrypted keys."
								: "Set a password in Authentication settings to start encrypting your stored keys."}
					</span>
				</div>

				{vaultStatus === "sealed" ? (
					<div style={{ display: "flex", flexDirection: "column", gap: "12px" }}>
						<form onSubmit={onUnlockPw} style={{ display: "flex", flexDirection: "column", gap: "6px" }}>
							<div className="text-xs text-[var(--muted)]">Unlock with password</div>
							<div style={{ display: "flex", gap: "8px", alignItems: "center" }}>
								<input
									type="password"
									className="provider-key-input"
									style={{ flex: 1 }}
									value={unlockPw}
									onInput={(e: Event) => setUnlockPw(targetValue(e))}
									placeholder="Your password"
								/>
								<button type="submit" className="provider-btn" disabled={unlockingPw || !unlockPw.trim()}>
									{unlockingPw ? "Unlocking\u2026" : "Unlock"}
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
									onInput={(e: Event) => setRecoveryKey(targetValue(e))}
									placeholder="XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX"
								/>
								<button type="submit" className="provider-btn" disabled={unlockingRk || !recoveryKey.trim()}>
									{unlockingRk ? "Unlocking\u2026" : "Unlock"}
								</button>
							</div>
						</form>
						<StatusMessage error={err} success={msg} />
					</div>
				) : null}

				{vaultStatus === "uninitialized" ? (
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
			</div>
		</div>
	);
}
