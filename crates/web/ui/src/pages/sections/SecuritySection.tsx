// ── Security section ─────────────────────────────────────────

import type { VNode } from "preact";
import { useEffect, useState } from "preact/hooks";
import { DangerZone, EmptyState, ListItem, Loading } from "../../components/forms";
import { refresh as refreshGon } from "../../gon";
import { detectPasskeyName } from "../../passkey-detect";
import { targetValue } from "../../typed-events";
import { prepareCreationOptions } from "../../webauthn-helpers";
import { rerender } from "./_shared";

// ── b64/buf helpers (used by passkey registration) ──────────

export function bufToB64(buf: ArrayBuffer): string {
	const bytes = new Uint8Array(buf);
	let str = "";
	for (const b of bytes) str += String.fromCharCode(b);
	return btoa(str).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

interface PasskeyEntry {
	id: string;
	name: string;
	created_at?: string;
}

interface ApiKeyEntry {
	id: string;
	label: string;
	key_prefix?: string;
	created_at?: string;
	scopes?: string[];
}

interface AkScopes {
	"operator.read": boolean;
	"operator.write": boolean;
	"operator.approvals": boolean;
	"operator.pairing": boolean;
	[key: string]: boolean;
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Large component managing auth, passwords, passkeys, and API keys
export function SecuritySection(): VNode {
	const [authDisabled, setAuthDisabled] = useState(false);
	const [localhostOnly, setLocalhostOnly] = useState(false);
	const [hasPassword, setHasPassword] = useState(true);
	const [hasPasskeys, setHasPasskeys] = useState(false);
	const [setupComplete, setSetupComplete] = useState(false);
	const [authLoading, setAuthLoading] = useState(true);

	const [curPw, setCurPw] = useState("");
	const [newPw, setNewPw] = useState("");
	const [confirmPw, setConfirmPw] = useState("");
	const [pwMsg, setPwMsg] = useState<string | null>(null);
	const [pwErr, setPwErr] = useState<string | null>(null);
	const [pwSaving, setPwSaving] = useState(false);
	const [pwAwaitingReauth, setPwAwaitingReauth] = useState(false);
	const [pwRecoveryKey, setPwRecoveryKey] = useState<string | null>(null);
	const [pwRecoveryCopied, setPwRecoveryCopied] = useState(false);

	const [passkeys, setPasskeys] = useState<PasskeyEntry[]>([]);
	const [pkName, setPkName] = useState("");
	const [pkMsg, setPkMsg] = useState<string | null>(null);
	const [pkLoading, setPkLoading] = useState(true);
	const [editingPk, setEditingPk] = useState<string | null>(null);
	const [editingPkName, setEditingPkName] = useState("");
	const [passkeyOrigins, setPasskeyOrigins] = useState<string[]>([]);
	const [passkeyHostUpdateHosts, setPasskeyHostUpdateHosts] = useState<string[]>([]);

	const [apiKeys, setApiKeys] = useState<ApiKeyEntry[]>([]);
	const [akLabel, setAkLabel] = useState("");
	const [akNew, setAkNew] = useState<string | null>(null);
	const [akLoading, setAkLoading] = useState(true);
	const [akFullAccess, setAkFullAccess] = useState(true);
	const [akScopes, setAkScopes] = useState<AkScopes>({
		"operator.read": false,
		"operator.write": false,
		"operator.approvals": false,
		"operator.pairing": false,
	});

	function notifyAuthStatusChanged(): void {
		window.dispatchEvent(new CustomEvent("moltis:auth-status-changed"));
	}

	function deferNextPasswordChangedRedirect(): void {
		window.__moltisSuppressNextPasswordChangedRedirect = true;
	}

	function clearPasswordChangedRedirectDeferral(): void {
		window.__moltisSuppressNextPasswordChangedRedirect = false;
	}

	function refreshPasskeyHostStatus(): Promise<void> {
		return fetch("/api/auth/status")
			.then((r) => (r.ok ? r.json() : null))
			.then((status: { passkey_host_update_hosts?: string[]; passkey_origins?: string[] } | null) => {
				if (Array.isArray(status?.passkey_host_update_hosts))
					setPasskeyHostUpdateHosts(status?.passkey_host_update_hosts);
				if (Array.isArray(status?.passkey_origins)) setPasskeyOrigins(status?.passkey_origins);
			});
	}

	function reloadIfAuthNowRequiresLogin({ reload = true } = {}): Promise<boolean> {
		return fetch("/api/auth/status")
			.then((r) => (r.ok ? r.json() : null))
			.then((d: { auth_disabled?: boolean; setup_required?: boolean; authenticated?: boolean } | null) => {
				const mustLogin = !!(d && d.auth_disabled === false && d.setup_required === false && d.authenticated === false);
				if (mustLogin && reload) {
					window.location.reload();
					return true;
				}
				return mustLogin;
			})
			.catch(() => false);
	}

	useEffect(() => {
		fetch("/api/auth/status")
			.then((r) => (r.ok ? r.json() : null))
			.then(
				(
					d: {
						auth_disabled?: boolean;
						localhost_only?: boolean;
						has_password?: boolean;
						has_passkeys?: boolean;
						setup_complete?: boolean;
						passkey_origins?: string[];
						passkey_host_update_hosts?: string[];
					} | null,
				) => {
					if (typeof d?.auth_disabled === "boolean") setAuthDisabled(d.auth_disabled);
					if (typeof d?.localhost_only === "boolean") setLocalhostOnly(d.localhost_only);
					if (typeof d?.has_password === "boolean") setHasPassword(d.has_password);
					if (typeof d?.has_passkeys === "boolean") setHasPasskeys(d.has_passkeys);
					if (typeof d?.setup_complete === "boolean") setSetupComplete(d.setup_complete);
					if (Array.isArray(d?.passkey_origins)) setPasskeyOrigins(d?.passkey_origins);
					if (Array.isArray(d?.passkey_host_update_hosts)) setPasskeyHostUpdateHosts(d?.passkey_host_update_hosts);
					setAuthLoading(false);
					rerender();
				},
			)
			.catch(() => {
				setAuthLoading(false);
				rerender();
			});
		fetch("/api/auth/passkeys")
			.then((r) => (r.ok ? r.json() : { passkeys: [] }))
			.then((d: { passkeys?: PasskeyEntry[] }) => {
				setPasskeys(d.passkeys || []);
				setHasPasskeys((d.passkeys || []).length > 0);
				setPkLoading(false);
				rerender();
			})
			.catch(() => setPkLoading(false));
		fetch("/api/auth/api-keys")
			.then((r) => (r.ok ? r.json() : { api_keys: [] }))
			.then((d: { api_keys?: ApiKeyEntry[] }) => {
				setApiKeys(d.api_keys || []);
				setAkLoading(false);
				rerender();
			})
			.catch(() => setAkLoading(false));
	}, []);

	function onChangePw(e: Event): void {
		e.preventDefault();
		setPwErr(null);
		setPwMsg(null);
		if (newPw.length < 12) {
			setPwErr("New password must be at least 12 characters.");
			return;
		}
		if (newPw !== confirmPw) {
			setPwErr("Passwords do not match.");
			return;
		}
		setPwSaving(true);
		setPwAwaitingReauth(false);
		const settingFirstPassword = !hasPassword;
		if (settingFirstPassword) deferNextPasswordChangedRedirect();
		const payload: { new_password: string; current_password?: string } = { new_password: newPw };
		if (hasPassword) payload.current_password = curPw;
		fetch("/api/auth/password/change", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify(payload),
		})
			.then((r) => {
				if (!r.ok) {
					return r.text().then((t) => {
						clearPasswordChangedRedirectDeferral();
						setPwErr(t);
						setPwSaving(false);
						setPwAwaitingReauth(false);
						rerender();
					});
				}

				return r.json().then((data: { recovery_key?: string }) => {
					const hasRecoveryKey = !!data.recovery_key;
					setPwMsg(hasPassword ? "Password changed." : "Password set.");
					setCurPw("");
					setNewPw("");
					setConfirmPw("");
					setHasPassword(true);
					setSetupComplete(true);
					setAuthDisabled(false);
					if (hasRecoveryKey) {
						setPwRecoveryKey(data.recovery_key!);
						refreshGon();
					}
					return reloadIfAuthNowRequiresLogin({ reload: !hasRecoveryKey }).then((requiresLoginOrReloaded) => {
						if (hasRecoveryKey && requiresLoginOrReloaded) {
							setPwAwaitingReauth(true);
							setPwMsg("Password set. Save the recovery key, then continue to sign in.");
							setPwSaving(false);
							rerender();
							return;
						}
						clearPasswordChangedRedirectDeferral();
						setPwAwaitingReauth(false);
						if (!requiresLoginOrReloaded) notifyAuthStatusChanged();
						setPwSaving(false);
						rerender();
					});
				});
			})
			.catch((err: Error) => {
				clearPasswordChangedRedirectDeferral();
				setPwErr(err.message);
				setPwSaving(false);
				setPwAwaitingReauth(false);
				rerender();
			});
	}

	function onAddPasskey(): void {
		setPkMsg(null);
		if (/^\d+\.\d+\.\d+\.\d+$/.test(location.hostname) || location.hostname.startsWith("[")) {
			setPkMsg(`Passkeys require a domain name. Use localhost instead of ${location.hostname}`);
			rerender();
			return;
		}
		let requestedRpId: string | null = null;
		fetch("/api/auth/passkey/register/begin", { method: "POST" })
			.then((r) => r.json())
			.then((data: { options: { publicKey: Record<string, unknown> }; challenge_id: string }) => {
				const pk = data.options.publicKey;
				requestedRpId = (pk.rp as { id?: string })?.id || null;
				const publicKey = prepareCreationOptions(pk);
				return navigator.credentials
					.create({ publicKey })
					.then((cred) => ({ cred: cred as PublicKeyCredential, challengeId: data.challenge_id }));
			})
			.then(({ cred, challengeId }: { cred: PublicKeyCredential; challengeId: string }) => {
				const response = cred.response as AuthenticatorAttestationResponse;
				const body = {
					challenge_id: challengeId,
					name: pkName.trim() || detectPasskeyName(cred),
					credential: {
						id: cred.id,
						rawId: bufToB64(cred.rawId),
						type: cred.type,
						response: {
							attestationObject: bufToB64(response.attestationObject),
							clientDataJSON: bufToB64(response.clientDataJSON),
						},
					},
				};
				return fetch("/api/auth/passkey/register/finish", {
					method: "POST",
					headers: { "Content-Type": "application/json" },
					body: JSON.stringify(body),
				});
			})
			.then((r) => {
				if (r.ok) {
					setPkName("");
					return reloadIfAuthNowRequiresLogin().then((reloaded) => {
						if (reloaded) return;
						return fetch("/api/auth/passkeys")
							.then((r2) => r2.json())
							.then((d: { passkeys?: PasskeyEntry[] }) => {
								setPasskeys(d.passkeys || []);
								setHasPasskeys((d.passkeys || []).length > 0);
								setSetupComplete(true);
								setAuthDisabled(false);
								return refreshPasskeyHostStatus().then(() => {
									setPkMsg("Passkey added.");
									notifyAuthStatusChanged();
									rerender();
								});
							});
					});
				}
				return r.text().then((t) => {
					setPkMsg(t);
					rerender();
				});
			})
			.catch((err: Error) => {
				let msg = err.message || "Failed to add passkey";
				if (requestedRpId) {
					msg += ` (RPID: "${requestedRpId}", current origin: "${location.origin}")`;
				}
				setPkMsg(msg);
				rerender();
			});
	}

	function onStartRename(id: string, currentName: string): void {
		setEditingPk(id);
		setEditingPkName(currentName);
		rerender();
	}

	function onCancelRename(): void {
		setEditingPk(null);
		setEditingPkName("");
		rerender();
	}

	function onConfirmRename(id: string): void {
		const name = editingPkName.trim();
		if (!name) return;
		fetch(`/api/auth/passkeys/${id}`, {
			method: "PATCH",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ name }),
		})
			.then(() => fetch("/api/auth/passkeys").then((r) => r.json()))
			.then((d: { passkeys?: PasskeyEntry[] }) => {
				setPasskeys(d.passkeys || []);
				setEditingPk(null);
				setEditingPkName("");
				rerender();
			});
	}

	function onRemovePasskey(id: string): void {
		fetch(`/api/auth/passkeys/${id}`, { method: "DELETE" })
			.then(() => fetch("/api/auth/passkeys").then((r) => r.json()))
			.then((d: { passkeys?: PasskeyEntry[] }) => {
				setPasskeys(d.passkeys || []);
				setHasPasskeys((d.passkeys || []).length > 0);
				return refreshPasskeyHostStatus().then(() => {
					notifyAuthStatusChanged();
					rerender();
				});
			});
	}

	function onCreateApiKey(): void {
		if (!akLabel.trim()) return;
		setAkNew(null);
		let scopes: string[] | null = null;
		if (!akFullAccess) {
			scopes = Object.entries(akScopes)
				.filter(([, v]) => v)
				.map(([k]) => k);
			if (scopes.length === 0) {
				return;
			}
		}
		fetch("/api/auth/api-keys", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ label: akLabel.trim(), scopes }),
		})
			.then((r) => r.json())
			.then((d: { key?: string }) => {
				setAkNew(d.key || null);
				setAkLabel("");
				setAkFullAccess(true);
				setAkScopes({
					"operator.read": false,
					"operator.write": false,
					"operator.approvals": false,
					"operator.pairing": false,
				});
				return fetch("/api/auth/api-keys").then((r2) => r2.json());
			})
			.then((d: { api_keys?: ApiKeyEntry[] }) => {
				setApiKeys(d.api_keys || []);
				rerender();
			})
			.catch(() => rerender());
	}

	function toggleScope(scope: string): void {
		setAkScopes((prev) => ({ ...prev, [scope]: !prev[scope] }));
		rerender();
	}

	function onRevokeApiKey(id: string): void {
		fetch(`/api/auth/api-keys/${id}`, { method: "DELETE" })
			.then(() => fetch("/api/auth/api-keys").then((r) => r.json()))
			.then((d: { api_keys?: ApiKeyEntry[] }) => {
				setApiKeys(d.api_keys || []);
				rerender();
			});
	}

	const [resetConfirm, setResetConfirm] = useState(false);
	const [resetBusy, setResetBusy] = useState(false);

	function onResetAuth(): void {
		if (!resetConfirm) {
			setResetConfirm(true);
			rerender();
			return;
		}
		setResetBusy(true);
		rerender();
		fetch("/api/auth/reset", { method: "POST" })
			.then((r) => {
				if (r.ok) {
					window.location.reload();
				} else {
					return r.text().then((t) => {
						setPwErr(t);
						setResetConfirm(false);
						setResetBusy(false);
						rerender();
					});
				}
			})
			.catch((err: Error) => {
				setPwErr(err.message);
				setResetConfirm(false);
				setResetBusy(false);
				rerender();
			});
	}

	if (authLoading) {
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<h2 className="text-lg font-medium text-[var(--text-strong)]">Authentication</h2>
				<Loading />
			</div>
		);
	}

	if (authDisabled && !localhostOnly) {
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<h2 className="text-lg font-medium text-[var(--text-strong)]">Authentication</h2>
				<div
					style={{
						maxWidth: "600px",
						padding: "12px 16px",
						borderRadius: "6px",
						border: "1px solid var(--error)",
						background: "color-mix(in srgb, var(--error) 5%, transparent)",
					}}
				>
					<strong style={{ color: "var(--error)" }}>Authentication is disabled</strong>
					<p className="text-xs text-[var(--muted)]" style={{ margin: "8px 0 0" }}>
						Anyone with network access can control moltis and your computer. Set up a password to protect your instance.
					</p>
					<button
						type="button"
						className="provider-btn"
						style={{ marginTop: "10px" }}
						onClick={() => {
							window.location.assign("/onboarding");
						}}
					>
						Set up authentication
					</button>
				</div>
			</div>
		);
	}

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<h2 className="text-lg font-medium text-[var(--text-strong)]">Authentication</h2>

			{authDisabled && localhostOnly ? (
				<div
					style={{
						maxWidth: "600px",
						padding: "12px 16px",
						borderRadius: "6px",
						border: "1px solid var(--error)",
						background: "color-mix(in srgb, var(--error) 5%, transparent)",
					}}
				>
					<strong style={{ color: "var(--error)" }}>Authentication is disabled</strong>
					<p className="text-xs text-[var(--muted)]" style={{ margin: "8px 0 0" }}>
						Localhost-only access is safe, but localhost bypass is active. Until you add a password or passkey, this
						browser has full access and Sign out has no effect. Add credentials below to require login on localhost and
						before exposing Moltis to your network.
					</p>
				</div>
			) : null}

			{localhostOnly && !hasPassword && !hasPasskeys && !authDisabled ? (
				<div className="alert-info-text max-w-form">
					<span className="alert-label-info">Note: </span>
					Localhost bypass is active. Until you add a password or passkey, this browser has full access and Sign out has
					no effect. Add credentials to require login on localhost and before exposing Moltis to your network.
				</div>
			) : null}

			{/* Password */}
			<div style={{ maxWidth: "600px" }}>
				<h3 className="text-sm font-medium text-[var(--text-strong)]" style={{ marginBottom: "8px" }}>
					{hasPassword ? "Change Password" : "Set Password"}
				</h3>
				<form onSubmit={onChangePw}>
					<div style={{ display: "flex", flexDirection: "column", gap: "8px", marginBottom: "10px" }}>
						{hasPassword ? (
							<div>
								<div className="text-xs text-[var(--muted)]" style={{ marginBottom: "4px" }}>
									Current password
								</div>
								<input
									type="password"
									className="provider-key-input"
									style={{ width: "100%" }}
									value={curPw}
									onInput={(e: Event) => setCurPw(targetValue(e))}
								/>
							</div>
						) : null}
						<div>
							<div className="text-xs text-[var(--muted)]" style={{ marginBottom: "4px" }}>
								{hasPassword ? "New password" : "Password"}
							</div>
							<input
								type="password"
								className="provider-key-input"
								style={{ width: "100%" }}
								value={newPw}
								onInput={(e: Event) => setNewPw(targetValue(e))}
								placeholder="At least 12 characters"
							/>
						</div>
						<div>
							<div className="text-xs text-[var(--muted)]" style={{ marginBottom: "4px" }}>
								Confirm {hasPassword ? "new " : ""}password
							</div>
							<input
								type="password"
								className="provider-key-input"
								style={{ width: "100%" }}
								value={confirmPw}
								onInput={(e: Event) => setConfirmPw(targetValue(e))}
							/>
						</div>
					</div>
					<div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
						<button type="submit" className="provider-btn" disabled={pwSaving}>
							{pwSaving
								? hasPassword
									? "Changing\u2026"
									: "Setting\u2026"
								: hasPassword
									? "Change password"
									: "Set password"}
						</button>
						{pwMsg ? (
							<span className="text-xs" style={{ color: "var(--accent)" }}>
								{pwMsg}
							</span>
						) : null}
						{pwErr ? (
							<span className="text-xs" style={{ color: "var(--error)" }}>
								{pwErr}
							</span>
						) : null}
					</div>
				</form>
				{pwRecoveryKey ? (
					<div
						style={{
							marginTop: "12px",
							padding: "12px 16px",
							borderRadius: "6px",
							border: "1px solid var(--border)",
							background: "var(--bg)",
						}}
					>
						<div className="text-xs text-[var(--muted)]" style={{ marginBottom: "4px" }}>
							Vault initialized {"\u2014"} save this recovery key
						</div>
						<code
							className="select-all break-all"
							style={{
								fontFamily: "var(--font-mono)",
								fontSize: ".8rem",
								color: "var(--text-strong)",
								display: "block",
								lineHeight: 1.5,
							}}
						>
							{pwRecoveryKey}
						</code>
						<div style={{ display: "flex", alignItems: "center", gap: "8px", marginTop: "8px" }}>
							<button
								type="button"
								className="provider-btn provider-btn-secondary"
								onClick={() => {
									navigator.clipboard.writeText(pwRecoveryKey).then(() => {
										setPwRecoveryCopied(true);
										setTimeout(() => {
											setPwRecoveryCopied(false);
											rerender();
										}, 2000);
										rerender();
									});
								}}
							>
								{pwRecoveryCopied ? "Copied!" : "Copy"}
							</button>
							{pwAwaitingReauth ? (
								<button
									type="button"
									className="provider-btn"
									onClick={() => {
										clearPasswordChangedRedirectDeferral();
										window.location.assign("/login");
									}}
								>
									Continue to sign in
								</button>
							) : null}
						</div>
						<div className="text-xs" style={{ color: "var(--error)", marginTop: "8px" }}>
							This key will not be shown again. You need it to unlock the vault if you forget your password.
						</div>
					</div>
				) : null}
			</div>

			{/* Passkeys */}
			<div style={{ maxWidth: "600px", borderTop: "1px solid var(--border)", paddingTop: "16px" }}>
				<h3 className="text-sm font-medium text-[var(--text-strong)]" style={{ marginBottom: "8px" }}>
					Passkeys
				</h3>
				{passkeyOrigins.length > 1 && (
					<div className="text-xs text-[var(--muted)]" style={{ marginBottom: "8px" }}>
						Passkeys will work when visiting: {passkeyOrigins.map((o) => o.replace(/^https?:\/\//, "")).join(", ")}
					</div>
				)}
				{hasPasskeys && passkeyHostUpdateHosts.length > 0 ? (
					<div className="alert-warning-text max-w-form" style={{ marginBottom: "8px" }}>
						<span className="alert-label-warning">Passkey update needed: </span>
						New host detected ({passkeyHostUpdateHosts.join(", ")}). Sign in with your password on that host, then
						register a new passkey there.
					</div>
				) : null}
				{pkLoading ? (
					<Loading />
				) : (
					<>
						{passkeys.length > 0 ? (
							<div style={{ display: "flex", flexDirection: "column", gap: "6px", marginBottom: "12px" }}>
								{passkeys.map((pk) =>
									editingPk === pk.id ? (
										<div className="provider-item" style={{ marginBottom: 0 }} key={pk.id}>
											<form
												style={{ display: "flex", alignItems: "center", gap: "6px", flex: 1 }}
												onSubmit={(e: Event) => {
													e.preventDefault();
													onConfirmRename(pk.id);
												}}
											>
												<input
													type="text"
													className="provider-key-input"
													value={editingPkName}
													onInput={(e: Event) => setEditingPkName(targetValue(e))}
													style={{ flex: 1 }}
												/>
												<button type="submit" className="provider-btn provider-btn-sm">
													Save
												</button>
												<button
													type="button"
													className="provider-btn provider-btn-sm provider-btn-secondary"
													onClick={onCancelRename}
												>
													Cancel
												</button>
											</form>
										</div>
									) : (
										<ListItem
											key={pk.id}
											name={pk.name}
											meta={<time dateTime={pk.created_at}>{pk.created_at}</time>}
											actions={[
												<button
													key="rename"
													className="provider-btn provider-btn-sm provider-btn-secondary"
													onClick={() => onStartRename(pk.id, pk.name)}
												>
													Rename
												</button>,
												<button
													key="remove"
													className="provider-btn provider-btn-sm provider-btn-danger"
													onClick={() => onRemovePasskey(pk.id)}
												>
													Remove
												</button>,
											]}
										/>
									),
								)}
							</div>
						) : (
							<EmptyState message="No passkeys registered." />
						)}
						<div style={{ display: "flex", gap: "8px", alignItems: "center" }}>
							<input
								type="text"
								className="provider-key-input"
								value={pkName}
								onInput={(e: Event) => setPkName(targetValue(e))}
								placeholder="Passkey name (e.g. MacBook Touch ID)"
								style={{ flex: 1 }}
							/>
							<button type="button" className="provider-btn" onClick={onAddPasskey}>
								Add passkey
							</button>
						</div>
						{pkMsg ? (
							<div className="text-xs text-[var(--muted)]" style={{ marginTop: "6px" }}>
								{pkMsg}
							</div>
						) : null}
					</>
				)}
			</div>

			{/* API Keys */}
			<div style={{ maxWidth: "600px", borderTop: "1px solid var(--border)", paddingTop: "16px" }}>
				<h3 className="text-sm font-medium text-[var(--text-strong)]" style={{ marginBottom: "4px" }}>
					API Keys
				</h3>
				<p className="text-xs text-[var(--muted)] leading-relaxed" style={{ margin: "0 0 12px" }}>
					API keys authenticate external tools and scripts connecting to moltis over the WebSocket protocol. Pass the
					key as the <code style={{ fontFamily: "var(--font-mono)", fontSize: ".75rem" }}>api_key</code> field in the{" "}
					<code style={{ fontFamily: "var(--font-mono)", fontSize: ".75rem" }}>auth</code> object of the{" "}
					<code style={{ fontFamily: "var(--font-mono)", fontSize: ".75rem" }}>connect</code> handshake.
				</p>
				{akLoading ? (
					<Loading />
				) : (
					<>
						{akNew ? (
							<div
								style={{
									marginBottom: "12px",
									padding: "10px 12px",
									background: "var(--bg)",
									border: "1px solid var(--border)",
									borderRadius: "6px",
								}}
							>
								<div className="text-xs text-[var(--muted)]" style={{ marginBottom: "4px" }}>
									Copy this key now. It won't be shown again.
								</div>
								<code
									style={{
										fontFamily: "var(--font-mono)",
										fontSize: ".78rem",
										wordBreak: "break-all",
										color: "var(--text-strong)",
									}}
								>
									{akNew}
								</code>
							</div>
						) : null}
						{apiKeys.length > 0 ? (
							<div style={{ display: "flex", flexDirection: "column", gap: "6px", marginBottom: "12px" }}>
								{apiKeys.map((ak) => (
									<ListItem
										key={ak.id}
										name={ak.label}
										meta={
											<span style={{ display: "flex", gap: "12px", flexWrap: "wrap" }}>
												<span style={{ fontFamily: "var(--font-mono)" }}>{ak.key_prefix}...</span>
												<span>
													<time dateTime={ak.created_at}>{ak.created_at}</time>
												</span>
												{ak.scopes ? (
													<span style={{ color: "var(--accent)" }}>{ak.scopes.join(", ")}</span>
												) : (
													<span style={{ color: "var(--accent)" }}>Full access</span>
												)}
											</span>
										}
										actions={
											<button className="provider-btn provider-btn-danger" onClick={() => onRevokeApiKey(ak.id)}>
												Revoke
											</button>
										}
									/>
								))}
							</div>
						) : (
							<EmptyState message="No API keys." />
						)}
						<div style={{ display: "flex", flexDirection: "column", gap: "10px" }}>
							<div style={{ display: "flex", gap: "8px", alignItems: "center" }}>
								<input
									type="text"
									className="provider-key-input"
									value={akLabel}
									onInput={(e: Event) => setAkLabel(targetValue(e))}
									placeholder="Key label (e.g. CLI tool)"
									style={{ flex: 1 }}
								/>
							</div>
							<div>
								<label style={{ display: "flex", alignItems: "center", gap: "6px", cursor: "pointer" }}>
									<input
										type="checkbox"
										checked={akFullAccess}
										onChange={() => {
											setAkFullAccess(!akFullAccess);
											rerender();
										}}
									/>
									<span className="text-xs text-[var(--text)]">Full access (all permissions)</span>
								</label>
							</div>
							{akFullAccess ? null : (
								<div style={{ paddingLeft: "20px", display: "flex", flexDirection: "column", gap: "6px" }}>
									<div className="text-xs text-[var(--muted)]" style={{ marginBottom: "2px" }}>
										Select permissions:
									</div>
									<label style={{ display: "flex", alignItems: "center", gap: "6px", cursor: "pointer" }}>
										<input
											type="checkbox"
											checked={akScopes["operator.read"]}
											onChange={() => toggleScope("operator.read")}
										/>
										<span className="text-xs text-[var(--text)]">operator.read</span>
										<span className="text-xs text-[var(--muted)]">{"\u2014"} View data and status</span>
									</label>
									<label style={{ display: "flex", alignItems: "center", gap: "6px", cursor: "pointer" }}>
										<input
											type="checkbox"
											checked={akScopes["operator.write"]}
											onChange={() => toggleScope("operator.write")}
										/>
										<span className="text-xs text-[var(--text)]">operator.write</span>
										<span className="text-xs text-[var(--muted)]">{"\u2014"} Create, update, delete</span>
									</label>
									<label style={{ display: "flex", alignItems: "center", gap: "6px", cursor: "pointer" }}>
										<input
											type="checkbox"
											checked={akScopes["operator.approvals"]}
											onChange={() => toggleScope("operator.approvals")}
										/>
										<span className="text-xs text-[var(--text)]">operator.approvals</span>
										<span className="text-xs text-[var(--muted)]">{"\u2014"} Handle exec approvals</span>
									</label>
									<label style={{ display: "flex", alignItems: "center", gap: "6px", cursor: "pointer" }}>
										<input
											type="checkbox"
											checked={akScopes["operator.pairing"]}
											onChange={() => toggleScope("operator.pairing")}
										/>
										<span className="text-xs text-[var(--text)]">operator.pairing</span>
										<span className="text-xs text-[var(--muted)]">{"\u2014"} Device/node pairing</span>
									</label>
								</div>
							)}
							<div>
								<button
									type="button"
									className="provider-btn"
									onClick={onCreateApiKey}
									disabled={!(akLabel.trim() && (akFullAccess || Object.values(akScopes).some((v) => v)))}
								>
									Generate key
								</button>
							</div>
						</div>
					</>
				)}
			</div>

			{/* Danger zone (only when auth has been set up) */}
			{setupComplete ? (
				<DangerZone>
					<div
						style={{
							padding: "12px 16px",
							border: "1px solid var(--error)",
							borderRadius: "6px",
							background: "color-mix(in srgb, var(--error) 5%, transparent)",
						}}
					>
						<strong className="text-sm" style={{ color: "var(--text-strong)" }}>
							Remove all authentication
						</strong>
						<p className="text-xs text-[var(--muted)]" style={{ margin: "6px 0 0" }}>
							If you know what you're doing, you can fully disable authentication. Anyone with network access will be
							able to access moltis and your computer. This removes your password, all passkeys, all API keys, and all
							sessions.
						</p>
						{resetConfirm ? (
							<div style={{ display: "flex", alignItems: "center", gap: "8px", marginTop: "10px" }}>
								<span className="text-xs" style={{ color: "var(--error)" }}>
									Are you sure? This cannot be undone.
								</span>
								<button
									type="button"
									className="provider-btn provider-btn-danger"
									disabled={resetBusy}
									onClick={onResetAuth}
								>
									{resetBusy ? "Removing\u2026" : "Yes, remove all auth"}
								</button>
								<button
									type="button"
									className="provider-btn"
									onClick={() => {
										setResetConfirm(false);
										rerender();
									}}
								>
									Cancel
								</button>
							</div>
						) : (
							<button
								type="button"
								className="provider-btn provider-btn-danger"
								style={{ marginTop: "10px" }}
								onClick={onResetAuth}
							>
								Remove all authentication
							</button>
						)}
					</div>
				</DangerZone>
			) : (
				""
			)}
		</div>
	);
}
