// ── Environment section ──────────────────────────────────────

import type { VNode } from "preact";
import { useEffect, useState } from "preact/hooks";
import {
	Badge,
	EmptyState,
	ListItem,
	Loading,
	SectionHeading,
	StatusMessage,
	SubHeading,
	useSaveState,
} from "../../components/forms";
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
	const save = useSaveState();
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
		save.reset();
		const key = newKey.trim();
		if (!key) {
			save.setError("Key is required.");
			rerender();
			return;
		}
		if (!/^[A-Za-z0-9_]+$/.test(key)) {
			save.setError("Key must contain only letters, digits, and underscores.");
			rerender();
			return;
		}
		save.setSaving(true);
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
					save.flashSaved();
					fetchEnvVars();
				} else {
					return r
						.json()
						.then((d: unknown) =>
							save.setError(
								localizedApiErrorMessage(d as Parameters<typeof localizedApiErrorMessage>[0], "Failed to save"),
							),
						);
				}
				save.setSaving(false);
				rerender();
			})
			.catch((err: Error) => {
				save.setError(err.message);
				save.setSaving(false);
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
			<SectionHeading title="Environment Variables" />
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
				<Loading />
			) : (
				<>
					{/* Existing variables */}
					<div style={{ maxWidth: "600px" }}>
						{envVars.length > 0 ? (
							<div style={{ display: "flex", flexDirection: "column", gap: "6px", marginBottom: "12px" }}>
								{envVars.map((v) =>
									updateId === v.id ? (
										<div className="provider-item" style={{ marginBottom: 0 }} key={v.id}>
											<form
												style={{ display: "flex", alignItems: "center", gap: "6px", flex: 1 }}
												onSubmit={(e: Event) => {
													e.preventDefault();
													onConfirmUpdate(v.key);
												}}
											>
												<code style={{ fontSize: "0.8rem", fontFamily: "var(--font-mono)" }}>{v.key}</code>
												{v.encrypted ? <Badge label="Encrypted" variant="configured" /> : <Badge label="Plaintext" />}
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
										</div>
									) : (
										<ListItem
											key={v.id}
											name={<span style={{ fontFamily: "var(--font-mono)", fontSize: ".8rem" }}>{v.key}</span>}
											badges={[
												v.encrypted ? <Badge label="Encrypted" variant="configured" /> : <Badge label="Plaintext" />,
											]}
											meta={
												<span style={{ display: "flex", gap: "12px" }}>
													<span>{"\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022"}</span>
													<time dateTime={v.updated_at}>{v.updated_at}</time>
												</span>
											}
											actions={[
												<button
													key="update"
													className="provider-btn provider-btn-sm"
													onClick={() => onStartUpdate(v.id)}
												>
													Update
												</button>,
												<button
													key="delete"
													className="provider-btn provider-btn-sm provider-btn-danger"
													onClick={() => onDelete(v.id)}
												>
													Delete
												</button>,
											]}
										/>
									),
								)}
							</div>
						) : (
							<EmptyState message="No environment variables set." />
						)}
					</div>

					{/* Add variable */}
					<div style={{ maxWidth: "600px", borderTop: "1px solid var(--border)", paddingTop: "16px" }}>
						<SubHeading title="Add Variable" />
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
								<button type="submit" className="provider-btn" disabled={save.saving || !newKey.trim()}>
									{save.saving ? "Saving\u2026" : "Add"}
								</button>
							</div>
							<StatusMessage error={save.error} success={save.saved ? "Variable saved." : null} />
						</form>
					</div>
				</>
			)}
		</div>
	);
}
