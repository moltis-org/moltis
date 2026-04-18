// ── Environment section ──────────────────────────────────────

import type { VNode } from "preact";
import { useEffect, useState } from "preact/hooks";
import * as gon from "../../gon";
import { localizedApiErrorMessage } from "../../helpers";
import { targetValue } from "../../typed-events";
import { rerender } from "./_shared";

interface EnvVar {
	id: string;
	key: string;
	encrypted?: boolean;
	updated_at?: string;
}

export function EnvironmentSection(): VNode {
	const [envVars, setEnvVars] = useState<EnvVar[]>([]);
	const [envLoading, setEnvLoading] = useState(true);
	const [newKey, setNewKey] = useState("");
	const [newValue, setNewValue] = useState("");
	const [envMsg, setEnvMsg] = useState<string | null>(null);
	const [envErr, setEnvErr] = useState<string | null>(null);
	const [saving, setSaving] = useState(false);
	const [updateId, setUpdateId] = useState<string | null>(null);
	const [updateValue, setUpdateValue] = useState("");

	function fetchEnvVars(): void {
		fetch("/api/env")
			.then((r) => (r.ok ? r.json() : { env_vars: [] }))
			.then((d: { env_vars?: EnvVar[] }) => {
				setEnvVars(d.env_vars || []);
				setEnvLoading(false);
				rerender();
			})
			.catch(() => {
				setEnvLoading(false);
				rerender();
			});
	}

	useEffect(() => {
		fetchEnvVars();
	}, []);

	function onAdd(e: Event): void {
		e.preventDefault();
		setEnvErr(null);
		setEnvMsg(null);
		const key = newKey.trim();
		if (!key) {
			setEnvErr("Key is required.");
			rerender();
			return;
		}
		if (!/^[A-Za-z0-9_]+$/.test(key)) {
			setEnvErr("Key must contain only letters, digits, and underscores.");
			rerender();
			return;
		}
		setSaving(true);
		rerender();
		fetch("/api/env", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ key, value: newValue }),
		})
			.then((r) => {
				if (r.ok) {
					setNewKey("");
					setNewValue("");
					setEnvMsg("Variable saved.");
					setTimeout(() => {
						setEnvMsg(null);
						rerender();
					}, 2000);
					fetchEnvVars();
				} else {
					return r
						.json()
						.then((d: unknown) =>
							setEnvErr(
								localizedApiErrorMessage(d as Parameters<typeof localizedApiErrorMessage>[0], "Failed to save"),
							),
						);
				}
				setSaving(false);
				rerender();
			})
			.catch((err: Error) => {
				setEnvErr(err.message);
				setSaving(false);
				rerender();
			});
	}

	function onDelete(id: string): void {
		fetch(`/api/env/${id}`, { method: "DELETE" }).then(() => fetchEnvVars());
	}

	function onStartUpdate(id: string): void {
		setUpdateId(id);
		setUpdateValue("");
		rerender();
	}

	function onCancelUpdate(): void {
		setUpdateId(null);
		setUpdateValue("");
		rerender();
	}

	function onConfirmUpdate(key: string): void {
		fetch("/api/env", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ key, value: updateValue }),
		}).then((r) => {
			if (r.ok) {
				setUpdateId(null);
				setUpdateValue("");
				fetchEnvVars();
			}
		});
	}

	const envVaultStatus = gon.get("vault_status");

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<h2 className="text-lg font-medium text-[var(--text-strong)]">Environment Variables</h2>
			<p className="text-xs text-[var(--muted)] leading-relaxed" style={{ maxWidth: "600px", margin: 0 }}>
				Environment variables are injected into sandbox command execution. Values are write-only and never displayed.
			</p>
			{envVaultStatus && envVaultStatus !== "disabled" ? (
				<div
					className="text-xs"
					style={{
						maxWidth: "600px",
						padding: "8px 12px",
						borderRadius: "6px",
						border: "1px solid var(--border)",
						background: "var(--bg)",
					}}
				>
					{envVaultStatus === "unsealed" ? (
						<>
							<span style={{ color: "var(--accent)" }}>Vault unlocked.</span> Your keys are stored encrypted.
						</>
					) : envVaultStatus === "sealed" ? (
						<>
							<span style={{ color: "var(--warning,var(--error))" }}>Vault locked.</span> Encrypted keys can{"\u2019"}t
							be read {"\u2014"} sandbox commands won{"\u2019"}t work.{" "}
							<a href="/settings/vault" style={{ color: "inherit", textDecoration: "underline" }}>
								Unlock in Encryption settings.
							</a>
						</>
					) : (
						<>
							<span className="text-[var(--muted)]">Vault not set up.</span>{" "}
							<a href="/settings/security" style={{ color: "inherit", textDecoration: "underline" }}>
								Set a password
							</a>{" "}
							to encrypt your stored keys.
						</>
					)}
				</div>
			) : null}

			{envLoading ? (
				<div className="text-xs text-[var(--muted)]">Loading{"\u2026"}</div>
			) : (
				<>
					{/* Existing variables */}
					<div style={{ maxWidth: "600px" }}>
						{envVars.length > 0 ? (
							<div style={{ display: "flex", flexDirection: "column", gap: "6px", marginBottom: "12px" }}>
								{envVars.map((v) => (
									<div className="provider-item" style={{ marginBottom: 0 }} key={v.id}>
										{updateId === v.id ? (
											<form
												style={{ display: "flex", alignItems: "center", gap: "6px", flex: 1 }}
												onSubmit={(e: Event) => {
													e.preventDefault();
													onConfirmUpdate(v.key);
												}}
											>
												<code style={{ fontSize: "0.8rem", fontFamily: "var(--font-mono)" }}>{v.key}</code>
												{v.encrypted ? (
													<span className="provider-item-badge configured">Encrypted</span>
												) : (
													<span className="provider-item-badge muted">Plaintext</span>
												)}
												<input
													type="password"
													className="provider-key-input"
													name="env_update_value"
													autoComplete="new-password"
													autoCorrect="off"
													autoCapitalize="off"
													spellcheck={false}
													value={updateValue}
													onInput={(e: Event) => setUpdateValue(targetValue(e))}
													placeholder="New value"
													style={{ flex: 1 }}
												/>
												<button type="submit" className="provider-btn">
													Save
												</button>
												<button type="button" className="provider-btn" onClick={onCancelUpdate}>
													Cancel
												</button>
											</form>
										) : (
											<>
												<div style={{ flex: 1, minWidth: 0 }}>
													<div
														className="provider-item-name"
														style={{ fontFamily: "var(--font-mono)", fontSize: ".8rem" }}
													>
														{v.key}
														{v.encrypted ? (
															<span className="provider-item-badge configured" style={{ marginLeft: "6px" }}>
																Encrypted
															</span>
														) : (
															<span className="provider-item-badge muted" style={{ marginLeft: "6px" }}>
																Plaintext
															</span>
														)}
													</div>
													<div
														style={{
															fontSize: ".7rem",
															color: "var(--muted)",
															marginTop: "2px",
															display: "flex",
															gap: "12px",
														}}
													>
														<span>{"\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022"}</span>
														<time dateTime={v.updated_at}>{v.updated_at}</time>
													</div>
												</div>
												<div style={{ display: "flex", gap: "4px" }}>
													<button className="provider-btn provider-btn-sm" onClick={() => onStartUpdate(v.id)}>
														Update
													</button>
													<button
														className="provider-btn provider-btn-sm provider-btn-danger"
														onClick={() => onDelete(v.id)}
													>
														Delete
													</button>
												</div>
											</>
										)}
									</div>
								))}
							</div>
						) : (
							<div className="text-xs text-[var(--muted)]" style={{ padding: "12px 0" }}>
								No environment variables set.
							</div>
						)}
					</div>

					{/* Add variable */}
					<div style={{ maxWidth: "600px", borderTop: "1px solid var(--border)", paddingTop: "16px" }}>
						<h3 className="text-sm font-medium text-[var(--text-strong)]" style={{ marginBottom: "8px" }}>
							Add Variable
						</h3>
						<form onSubmit={onAdd}>
							<div style={{ display: "flex", gap: "8px", flexWrap: "wrap" }}>
								<input
									type="text"
									className="provider-key-input"
									name="env_key"
									autoComplete="off"
									autoCorrect="off"
									autoCapitalize="off"
									spellcheck={false}
									value={newKey}
									onInput={(e: Event) => setNewKey(targetValue(e))}
									placeholder="KEY_NAME"
									style={{ flex: 1, minWidth: "120px", fontFamily: "var(--font-mono)", fontSize: ".8rem" }}
								/>
								<input
									type="password"
									className="provider-key-input"
									name="env_value"
									autoComplete="new-password"
									autoCorrect="off"
									autoCapitalize="off"
									spellcheck={false}
									value={newValue}
									onInput={(e: Event) => setNewValue(targetValue(e))}
									placeholder="Value"
									style={{ flex: 2, minWidth: "200px" }}
								/>
								<button type="submit" className="provider-btn" disabled={saving || !newKey.trim()}>
									{saving ? "Saving\u2026" : "Add"}
								</button>
							</div>
							{envMsg ? (
								<div className="text-xs" style={{ marginTop: "6px", color: "var(--accent)" }}>
									{envMsg}
								</div>
							) : null}
							{envErr ? (
								<div className="text-xs" style={{ marginTop: "6px", color: "var(--error)" }}>
									{envErr}
								</div>
							) : null}
						</form>
					</div>
				</>
			)}
		</div>
	);
}
