// ── Auth step (passkey + password setup) ─────────────────────

import type { VNode } from "preact";
import { useEffect, useState } from "preact/hooks";
import { t } from "../../i18n";
import { detectPasskeyName } from "../../passkey-detect";
import { targetValue } from "../../typed-events";
import { copyToClipboard } from "../../ui";
import { prepareCreationOptions } from "../../webauthn-helpers";
import { bufferToBase64, ErrorPanel, ensureWsConnected } from "../shared";

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: auth step handles passkey+password+code flows
export function AuthStep({ onNext, skippable }: { onNext: () => void; skippable: boolean }): VNode {
	const [method, setMethod] = useState<string | null>(null); // null | "passkey" | "password"
	const [password, setPassword] = useState("");
	const [confirm, setConfirm] = useState("");
	const [setupCode, setSetupCode] = useState("");
	const [passkeyName, setPasskeyName] = useState("");
	const [codeRequired, setCodeRequired] = useState(false);
	const [localhostOnly, setLocalhostOnly] = useState(false);
	const [webauthnAvailable, setWebauthnAvailable] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const [saving, setSaving] = useState(false);
	const [loading, setLoading] = useState(true);
	const [passkeyOrigins, setPasskeyOrigins] = useState<string[]>([]);
	const [passkeyDone, setPasskeyDone] = useState(false);
	const [optPw, setOptPw] = useState("");
	const [optPwConfirm, setOptPwConfirm] = useState("");
	const [optPwSaving, setOptPwSaving] = useState(false);
	const [recoveryKey, setRecoveryKey] = useState<string | null>(null);
	const [recoveryCopied, setRecoveryCopied] = useState(false);

	const isIpAddress = /^\d+\.\d+\.\d+\.\d+$/.test(location.hostname) || location.hostname.startsWith("[");
	const browserSupportsWebauthn = !!window.PublicKeyCredential;
	const passkeyEnabled = webauthnAvailable && browserSupportsWebauthn && !isIpAddress;

	const [setupComplete, setSetupComplete] = useState(false);

	useEffect(() => {
		fetch("/api/auth/status")
			.then((r) => r.json())
			.then(
				(data: {
					setup_code_required?: boolean;
					localhost_only?: boolean;
					webauthn_available?: boolean;
					passkey_origins?: string[];
					setup_complete?: boolean;
				}) => {
					if (data.setup_code_required) setCodeRequired(true);
					if (data.localhost_only) setLocalhostOnly(true);
					if (data.webauthn_available) setWebauthnAvailable(true);
					if (data.passkey_origins) setPasskeyOrigins(data.passkey_origins);
					if (data.setup_complete) setSetupComplete(true);
					setLoading(false);
				},
			)
			.catch(() => setLoading(false));
	}, []);

	// Pre-select passkey when available (easier than passwords)
	useEffect(() => {
		if (passkeyEnabled && method === null) setMethod("passkey");
	}, [passkeyEnabled]);

	// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: password+code validation
	function onPasswordSubmit(e: Event): void {
		e.preventDefault();
		setError(null);
		if (password.length > 0 || !localhostOnly) {
			if (password.length < 12) {
				setError("Password must be at least 12 characters.");
				return;
			}
			if (password !== confirm) {
				setError("Passwords do not match.");
				return;
			}
		}
		if (codeRequired && setupCode.trim().length === 0) {
			setError("Enter the setup code shown in the process log (stdout).");
			return;
		}
		setSaving(true);
		const body: Record<string, string> = password ? { password } : {};
		if (codeRequired) body.setup_code = setupCode.trim();
		fetch("/api/auth/setup", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify(body),
		})
			.then((r) => {
				if (r.ok) {
					ensureWsConnected();
					return r
						.json()
						.then((data: { recovery_key?: string }) => {
							if (data.recovery_key) {
								setRecoveryKey(data.recovery_key);
								setSaving(false);
							} else {
								onNext();
							}
						})
						.catch(() => onNext());
				} else {
					return r.text().then((txt: string) => {
						setError(txt || "Setup failed");
						setSaving(false);
					});
				}
			})
			.catch((err: Error) => {
				setError(err.message);
				setSaving(false);
			});
	}

	function onPasskeyRegister(): void {
		setError(null);
		if (codeRequired && setupCode.trim().length === 0) {
			setError("Enter the setup code shown in the process log (stdout).");
			return;
		}
		setSaving(true);
		const codeBody: Record<string, string> = codeRequired ? { setup_code: setupCode.trim() } : {};
		let requestedRpId: string | null = null;
		fetch("/api/auth/setup/passkey/register/begin", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify(codeBody),
		})
			.then((r) => {
				if (!r.ok)
					return r
						.text()
						.then((txt: string) => Promise.reject(new Error(txt || "Failed to start passkey registration")));
				return r.json();
			})
			.then((data: { options: Record<string, unknown>; challenge_id: string }) => {
				const pk = data.options.publicKey as Record<string, unknown>;
				requestedRpId = (pk.rp as Record<string, string>)?.id || null;
				const publicKey = prepareCreationOptions(pk);
				return navigator.credentials
					.create({ publicKey })
					.then((cred) => ({ cred: cred as PublicKeyCredential, challengeId: data.challenge_id }));
			})
			.then(({ cred, challengeId }) => {
				const attestation = cred.response as AuthenticatorAttestationResponse;
				const body: {
					challenge_id: string;
					name: string;
					credential: {
						id: string;
						rawId: string;
						type: string;
						response: { attestationObject: string; clientDataJSON: string };
					};
					setup_code?: string;
				} = {
					challenge_id: challengeId,
					name: passkeyName.trim() || detectPasskeyName(cred),
					credential: {
						id: cred.id,
						rawId: bufferToBase64(cred.rawId),
						type: cred.type,
						response: {
							attestationObject: bufferToBase64(attestation.attestationObject),
							clientDataJSON: bufferToBase64(attestation.clientDataJSON),
						},
					},
				};
				if (codeRequired) body.setup_code = setupCode.trim();
				return fetch("/api/auth/setup/passkey/register/finish", {
					method: "POST",
					headers: { "Content-Type": "application/json" },
					body: JSON.stringify(body),
				});
			})
			.then((r) => {
				if (r.ok) {
					ensureWsConnected();
					setSaving(false);
					setPasskeyDone(true);
				} else {
					return r.text().then((txt: string) => {
						setError(txt || "Passkey registration failed");
						setSaving(false);
					});
				}
			})
			.catch((err: Error & { name?: string }) => {
				if (err.name === "NotAllowedError") {
					setError("Passkey registration was cancelled.");
				} else {
					let msg = err.message || "Passkey registration failed";
					if (requestedRpId) {
						msg += ` (RPID: "${requestedRpId}", current origin: "${location.origin}")`;
					}
					setError(msg);
				}
				setSaving(false);
			});
	}

	function onOptionalPassword(e: Event): void {
		e.preventDefault();
		setError(null);
		if (optPw.length < 12) {
			setError("Password must be at least 12 characters.");
			return;
		}
		if (optPw !== optPwConfirm) {
			setError("Passwords do not match.");
			return;
		}
		setOptPwSaving(true);
		fetch("/api/auth/password/change", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ new_password: optPw }),
		})
			.then((r) => {
				if (r.ok) {
					ensureWsConnected();
					onNext();
				} else {
					return r.text().then((txt: string) => {
						setError(txt || "Failed to set password");
						setOptPwSaving(false);
					});
				}
			})
			.catch((err: Error) => {
				setError(err.message);
				setOptPwSaving(false);
			});
	}

	if (loading) {
		return <div className="text-sm text-[var(--muted)]">Checking authentication{"\u2026"}</div>;
	}

	// Setup already complete (passkeys/password configured) — let user proceed.
	if (setupComplete) {
		return (
			<div className="flex flex-col gap-4">
				<h2 className="text-lg font-medium text-[var(--text-strong)]">{t("onboarding:auth.secureYourInstance")}</h2>
				<div className="flex items-center gap-2 text-sm text-[var(--accent)]">
					<span className="icon icon-checkmark" />
					Authentication is already configured.
				</div>
				<div className="flex flex-wrap items-center gap-3 mt-1">
					<button
						key={`auth-${saving}`}
						type="button"
						className="provider-btn"
						onClick={() => {
							ensureWsConnected();
							onNext();
						}}
					>
						Next
					</button>
				</div>
			</div>
		);
	}

	// ── Recovery key display after vault initialization ────
	if (recoveryKey) {
		return (
			<div className="flex flex-col gap-4">
				<h2 className="text-lg font-medium text-[var(--text-strong)]">Secure your instance</h2>
				<div className="flex items-center gap-2 text-sm text-[var(--accent)]">
					<span className="icon icon-checkmark" />
					Password set and vault initialized
				</div>
				<div
					style={{
						maxWidth: "600px",
						padding: "12px 16px",
						borderRadius: "6px",
						border: "1px solid var(--border)",
						background: "var(--bg)",
					}}
				>
					<div className="text-xs text-[var(--muted)]" style={{ marginBottom: "8px" }}>
						Recovery key
					</div>
					<code
						className="select-all break-all"
						style={{
							fontFamily: "var(--font-mono)",
							fontSize: ".8rem",
							color: "var(--text-strong)",
							display: "block",
							lineHeight: "1.5",
						}}
					>
						{recoveryKey}
					</code>
					<div style={{ display: "flex", alignItems: "center", gap: "8px", marginTop: "10px" }}>
						<button
							type="button"
							className="provider-btn provider-btn-secondary"
							onClick={() => {
								copyToClipboard(
									recoveryKey ?? "",
									"",
									"Could not copy — please select and copy the key manually.",
								).then((ok) => {
									if (!ok) return;
									setRecoveryCopied(true);
									setTimeout(() => setRecoveryCopied(false), 2000);
								});
							}}
						>
							{recoveryCopied ? "Copied!" : "Copy"}
						</button>
					</div>
				</div>
				<div className="text-xs" style={{ color: "var(--error)", maxWidth: "600px" }}>
					Save this recovery key in a safe place. It will not be shown again. You need it to unlock the vault if you
					forget your password.
				</div>
				<div className="flex flex-wrap items-center gap-3 mt-1">
					<button type="button" className="provider-btn" onClick={onNext}>
						Continue
					</button>
				</div>
			</div>
		);
	}

	const passkeyDisabledReason = webauthnAvailable
		? browserSupportsWebauthn
			? isIpAddress
				? "Requires domain name"
				: null
			: "Browser not supported"
		: "Not available on this server";

	const originsHint =
		passkeyOrigins.length > 1 ? passkeyOrigins.map((o) => o.replace(/^https?:\/\//, "")).join(", ") : null;

	// ── After passkey registration: optional password ────────
	if (passkeyDone) {
		return (
			<div className="flex flex-col gap-4">
				<h2 className="text-lg font-medium text-[var(--text-strong)]">{t("onboarding:auth.secureYourInstance")}</h2>
				<div className="flex items-center gap-2 text-sm text-[var(--accent)]">
					<span className="icon icon-checkmark" />
					Passkey registered successfully!
				</div>
				<p className="text-xs text-[var(--muted)] leading-relaxed">
					Optionally set a password as a fallback for when passkeys aren't available.
				</p>
				<form onSubmit={onOptionalPassword} className="flex flex-col gap-3">
					<div>
						<label htmlFor="onboarding-passkey-password" className="text-xs text-[var(--muted)] mb-1 block">
							Password
						</label>
						<input
							id="onboarding-passkey-password"
							type="password"
							name="password"
							autoComplete="new-password"
							className="provider-key-input w-full"
							value={optPw}
							onInput={(e) => setOptPw(targetValue(e))}
							placeholder="At least 12 characters"
							autofocus
						/>
					</div>
					<div>
						<label htmlFor="onboarding-passkey-password-confirm" className="text-xs text-[var(--muted)] mb-1 block">
							Confirm password
						</label>
						<input
							id="onboarding-passkey-password-confirm"
							type="password"
							name="confirm_password"
							autoComplete="new-password"
							className="provider-key-input w-full"
							value={optPwConfirm}
							onInput={(e) => setOptPwConfirm(targetValue(e))}
							placeholder="Repeat password"
						/>
					</div>
					{error && <ErrorPanel message={error} />}
					<div className="flex flex-wrap items-center gap-3 mt-1">
						<button type="submit" className="provider-btn" disabled={optPwSaving}>
							{optPwSaving ? "Setting\u2026" : "Set password & continue"}
						</button>
						<button
							type="button"
							className="text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline"
							onClick={() => {
								ensureWsConnected();
								onNext();
							}}
						>
							Skip
						</button>
					</div>
				</form>
			</div>
		);
	}

	// ── Method selection ─────────────────────────────────────
	return (
		<div className="flex flex-col gap-4">
			<h2 className="text-lg font-medium text-[var(--text-strong)]">{t("onboarding:auth.secureYourInstance")}</h2>
			<p className="text-xs text-[var(--muted)] leading-relaxed">
				{localhostOnly
					? "Choose how to secure your instance, or skip for now. Setting a password also enables the encryption vault, which protects API keys and secrets stored in the database."
					: "Choose how to secure your instance."}
			</p>

			{codeRequired && (
				<div>
					<label className="text-xs text-[var(--muted)] mb-1 block">Setup code</label>
					<input
						type="text"
						className="provider-key-input w-full"
						inputMode="numeric"
						pattern="[0-9]*"
						value={setupCode}
						onInput={(e) => setSetupCode(targetValue(e))}
						placeholder="6-digit code from terminal"
					/>
					<div className="text-xs text-[var(--muted)] mt-1">Find this code in the moltis process log (stdout).</div>
				</div>
			)}

			<div className="flex flex-col gap-2">
				<div
					className={`backend-card ${method === "passkey" ? "selected" : ""} ${passkeyEnabled ? "" : "disabled"}`}
					onClick={passkeyEnabled ? () => setMethod("passkey") : undefined}
				>
					<div className="flex flex-wrap items-center justify-between gap-2">
						<span className="text-sm font-medium text-[var(--text)]">Passkey</span>
						<div className="flex flex-wrap gap-2 justify-end">
							{passkeyEnabled ? <span className="recommended-badge">Recommended</span> : null}
							{passkeyDisabledReason ? <span className="tier-badge">{passkeyDisabledReason}</span> : null}
						</div>
					</div>
					<div className="text-xs text-[var(--muted)] mt-1">Use Touch ID, Face ID, or a security key</div>
				</div>
				<div
					className={`backend-card ${method === "password" ? "selected" : ""}`}
					onClick={() => setMethod("password")}
				>
					<div className="flex flex-wrap items-center justify-between gap-2">
						<span className="text-sm font-medium text-[var(--text)]">Password</span>
					</div>
					<div className="text-xs text-[var(--muted)] mt-1">
						Set a password and enable the encryption vault for stored secrets
					</div>
				</div>
			</div>

			{method === "passkey" && (
				<div className="flex flex-col gap-3">
					<div>
						<label className="text-xs text-[var(--muted)] mb-1 block">Passkey name</label>
						<input
							type="text"
							className="provider-key-input w-full"
							value={passkeyName}
							onInput={(e) => setPasskeyName(targetValue(e))}
							placeholder="e.g. MacBook Touch ID (optional)"
						/>
					</div>
					{originsHint && (
						<div className="text-xs text-[var(--muted)]">Passkeys will work when visiting: {originsHint}</div>
					)}
					{error && <ErrorPanel message={error} />}
					<div className="flex flex-wrap items-center gap-3 mt-1">
						<button
							key={`pk-${saving}`}
							type="button"
							className="provider-btn"
							disabled={saving}
							onClick={onPasskeyRegister}
						>
							{saving ? "Registering\u2026" : "Register passkey"}
						</button>
						{skippable ? (
							<button
								type="button"
								className="text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline"
								onClick={onNext}
							>
								{t("common:actions.skip")}
							</button>
						) : null}
					</div>
				</div>
			)}

			{method === "password" && (
				<form onSubmit={onPasswordSubmit} className="flex flex-col gap-3">
					<div>
						<label htmlFor="onboarding-password" className="text-xs text-[var(--muted)] mb-1 block">
							Password{localhostOnly ? "" : " *"}
						</label>
						<input
							id="onboarding-password"
							type="password"
							name="password"
							autoComplete="new-password"
							className="provider-key-input w-full"
							value={password}
							onInput={(e) => setPassword(targetValue(e))}
							placeholder={localhostOnly ? "Optional on localhost" : "At least 12 characters"}
							autofocus
						/>
					</div>
					<div>
						<label htmlFor="onboarding-password-confirm" className="text-xs text-[var(--muted)] mb-1 block">
							Confirm password
						</label>
						<input
							id="onboarding-password-confirm"
							type="password"
							name="confirm_password"
							autoComplete="new-password"
							className="provider-key-input w-full"
							value={confirm}
							onInput={(e) => setConfirm(targetValue(e))}
							placeholder="Repeat password"
						/>
					</div>
					{error && <ErrorPanel message={error} />}
					<div className="flex flex-wrap items-center gap-3 mt-1">
						<button key={`pw-${saving}`} type="submit" className="provider-btn" disabled={saving}>
							{saving ? "Setting up\u2026" : localhostOnly && !password ? "Skip" : "Set password"}
						</button>
						{skippable ? (
							<button
								type="button"
								className="text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline"
								onClick={onNext}
							>
								{t("common:actions.skip")}
							</button>
						) : null}
					</div>
				</form>
			)}

			{method === null && (
				<div className="flex flex-wrap items-center gap-3 mt-1">
					{skippable ? (
						<button
							type="button"
							className="text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline"
							onClick={onNext}
						>
							{t("common:actions.skip")}
						</button>
					) : null}
				</div>
			)}
		</div>
	);
}
