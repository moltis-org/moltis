// ── Configuration + GraphQL sections ─────────────────────────

import type { VNode } from "preact";
import { useEffect, useState } from "preact/hooks";
import { Loading } from "../../components/forms";
import { sendRpc } from "../../helpers";
import { clearAllSessions } from "../../sessions";
import { connected } from "../../signals";
import { targetChecked, targetValue } from "../../typed-events";
import { showToast } from "../../ui";
import type { RpcResponse } from "./_shared";
import { rerender } from "./_shared";

export function GraphqlSection(): VNode {
	const [loadingConfig, setLoadingConfig] = useState(true);
	const [enabled, setEnabled] = useState(false);
	const [saving, setSaving] = useState(false);
	const [msg, setMsg] = useState<string | null>(null);
	const [err, setErr] = useState<string | null>(null);
	const origin = window.location.origin;
	const wsProtocol = window.location.protocol === "https:" ? "wss:" : "ws:";
	const httpEndpoint = `${origin}/graphql`;
	const wsEndpoint = `${wsProtocol}//${window.location.host}/graphql`;

	function loadGraphqlConfig(): void {
		if (!connected.value) {
			setLoadingConfig(true);
			return;
		}
		setLoadingConfig(true);
		sendRpc("graphql.config.get", {})
			.then((res: RpcResponse) => {
				if (res?.ok) {
					setEnabled((res.payload as { enabled?: boolean })?.enabled !== false);
					setErr(null);
				} else {
					setErr((res?.error as { message?: string })?.message || "Failed to load GraphQL config");
				}
				setLoadingConfig(false);
				rerender();
			})
			.catch((error: Error) => {
				setErr(error?.message || "Failed to load GraphQL config");
				setLoadingConfig(false);
				rerender();
			});
	}

	useEffect(() => {
		if (connected.value) {
			loadGraphqlConfig();
		} else {
			setLoadingConfig(true);
			setSaving(false);
			setMsg(null);
		}
	}, [connected.value]);

	function onToggle(nextEnabled: boolean): void {
		if (!connected.value) {
			setErr("WebSocket not connected");
			rerender();
			return;
		}
		setSaving(true);
		setMsg(null);
		setErr(null);
		rerender();

		sendRpc("graphql.config.set", { enabled: nextEnabled })
			.then((res: RpcResponse) => {
				setSaving(false);
				if (res?.ok) {
					const payload = res.payload as { enabled?: boolean; persisted?: boolean };
					setEnabled(payload?.enabled !== false);
					if (payload?.persisted === false) {
						setMsg("GraphQL updated for this runtime, but failed to persist to config. It may revert on restart.");
					}
				} else {
					setErr((res?.error as { message?: string })?.message || "Failed to update GraphQL setting");
				}
				rerender();
			})
			.catch((error: Error) => {
				setSaving(false);
				setErr(error?.message || "Failed to update GraphQL setting");
				rerender();
			});
	}

	if (!connected.value) {
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<Loading message="Connecting..." />
			</div>
		);
	}

	if (loadingConfig) {
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<Loading />
			</div>
		);
	}

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<div
				style={{
					maxWidth: "900px",
					padding: "12px 14px",
					borderRadius: "8px",
					border: "1px solid var(--border)",
					background: "var(--surface)",
				}}
			>
				<div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: "12px" }}>
					<div>
						<div className="text-sm font-medium text-[var(--text-strong)]">GraphQL server</div>
						{enabled ? (
							<div className="text-xs text-[var(--muted)]" style={{ marginTop: "8px" }}>
								<div>
									HTTP endpoint: <code>{httpEndpoint}</code>
								</div>
								<div style={{ marginTop: "2px" }}>
									WebSocket endpoint: <code>{wsEndpoint}</code>
								</div>
							</div>
						) : null}
					</div>
					<label id="graphqlToggleSwitch" className="toggle-switch">
						<input
							id="graphqlEnabledToggle"
							type="checkbox"
							checked={enabled}
							disabled={saving || loadingConfig || !connected.value}
							onChange={(e: Event) => onToggle(targetChecked(e))}
						/>
						<span className="toggle-slider" />
					</label>
				</div>
				{saving ? (
					<div className="text-xs text-[var(--muted)]" style={{ marginTop: "8px" }}>
						Applying...
					</div>
				) : null}
				{msg ? (
					<div className="text-xs text-[var(--ok)]" style={{ marginTop: "8px" }}>
						{msg}
					</div>
				) : null}
				{err ? (
					<div className="text-xs text-[var(--error)]" style={{ marginTop: "8px" }}>
						{err}
					</div>
				) : null}
			</div>

			{enabled ? (
				<div className="flex-1 min-h-0 overflow-hidden rounded-[var(--radius)] border border-[var(--border)] bg-[var(--surface)]">
					<iframe
						src="/graphql"
						className="h-full w-full border-0"
						title="GraphiQL Playground"
						allow="clipboard-write"
					/>
				</div>
			) : null}
		</div>
	);
}

export function ConfigSection(): VNode {
	const [toml, setToml] = useState("");
	const [configPath, setConfigPath] = useState("");
	const [configLoading, setConfigLoading] = useState(true);
	const [saving, setSaving] = useState(false);
	const [testing, setTesting] = useState(false);
	const [resettingTemplate, setResettingTemplate] = useState(false);
	const [restarting, setRestarting] = useState(false);
	const [msg, setMsg] = useState<string | null>(null);
	const [err, setErr] = useState<string | null>(null);
	const [warnings, setWarnings] = useState<string[]>([]);

	function fetchConfig(): void {
		setConfigLoading(true);
		rerender();
		fetch("/api/config")
			.then((r) => {
				if (!r.ok) {
					return r.text().then((text) => {
						try {
							const json = JSON.parse(text);
							return { error: json.error || `HTTP ${r.status}: ${r.statusText}` };
						} catch (_e) {
							return { error: `HTTP ${r.status}: ${r.statusText}` };
						}
					});
				}
				return r.json().catch(() => ({ error: "Invalid JSON response from server" }));
			})
			.then((d: { error?: string; toml?: string; path?: string }) => {
				if (d.error) {
					setErr(d.error);
				} else {
					setToml(d.toml || "");
					setConfigPath(d.path || "");
					setErr(null);
				}
				setConfigLoading(false);
				rerender();
			})
			.catch((fetchErr: Error) => {
				let errMsg = fetchErr.message || "Network error";
				if (errMsg.includes("pattern")) {
					errMsg = "Failed to connect to server. Please check if moltis is running.";
				}
				setErr(errMsg);
				setConfigLoading(false);
				rerender();
			});
	}

	useEffect(() => {
		fetchConfig();
	}, []);

	function onTest(e: Event): void {
		e.preventDefault();
		setTesting(true);
		setMsg(null);
		setErr(null);
		setWarnings([]);
		rerender();

		fetch("/api/config/validate", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ toml }),
		})
			.then((r) => r.json().catch(() => ({ error: "Invalid JSON response" })))
			.then((d: { valid?: boolean; error?: string; warnings?: string[] }) => {
				setTesting(false);
				if (d.valid) {
					setMsg("Configuration is valid.");
					setWarnings(d.warnings || []);
				} else {
					setErr(d.error || "Invalid configuration");
				}
				rerender();
			})
			.catch((fetchErr: Error) => {
				setTesting(false);
				let errMsg = fetchErr.message || "Network error";
				if (errMsg.includes("pattern")) {
					errMsg = "Failed to connect to server";
				}
				setErr(errMsg);
				rerender();
			});
	}

	function onSave(e: Event): void {
		e.preventDefault();
		setSaving(true);
		setMsg(null);
		setErr(null);
		setWarnings([]);
		rerender();

		fetch("/api/config", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ toml }),
		})
			.then((r) => r.json().catch(() => ({ error: "Invalid JSON response" })))
			.then((d: { ok?: boolean; error?: string }) => {
				setSaving(false);
				if (d.ok) {
					setMsg("Configuration saved. Restart required for changes to take effect.");
				} else {
					setErr(d.error || "Failed to save");
				}
				rerender();
			})
			.catch((fetchErr: Error) => {
				setSaving(false);
				let errMsg = fetchErr.message || "Network error";
				if (errMsg.includes("pattern")) {
					errMsg = "Failed to connect to server";
				}
				setErr(errMsg);
				rerender();
			});
	}

	function onRestart(): void {
		setRestarting(true);
		setMsg("Restarting moltis...");
		setErr(null);
		rerender();

		fetch("/api/restart", { method: "POST" })
			.then((r) =>
				r
					.json()
					.catch(() => ({}))
					.then((d: { error?: string }) => ({ status: r.status, data: d })),
			)
			.then(({ status, data }: { status: number; data: { error?: string } }) => {
				if (status >= 400 && data.error) {
					setRestarting(false);
					setErr(data.error);
					setMsg(null);
					rerender();
				} else {
					setTimeout(waitForRestart, 1000);
				}
			})
			.catch(() => {
				setTimeout(waitForRestart, 1000);
			});
	}

	function waitForRestart(): void {
		let attempts = 0;
		const maxAttempts = 30;

		function check(): void {
			attempts++;
			fetch("/api/gon", { method: "GET" })
				.then((r) => {
					if (r.ok) {
						window.location.reload();
					} else if (attempts < maxAttempts) {
						setTimeout(check, 1000);
					} else {
						setRestarting(false);
						setErr("Server did not come back up. Check if moltis is running.");
						rerender();
					}
				})
				.catch(() => {
					if (attempts < maxAttempts) {
						setTimeout(check, 1000);
					} else {
						setRestarting(false);
						setErr("Server did not come back up. Check if moltis is running.");
						rerender();
					}
				});
		}

		check();
	}

	function onReset(): void {
		fetchConfig();
		setMsg(null);
		setErr(null);
		setWarnings([]);
	}

	function onResetToTemplate(): void {
		if (
			!confirm(
				"Replace current config with the default template?\n\nThis will show all available options with documentation. Your current values will be lost unless you copy them first.",
			)
		) {
			return;
		}
		setResettingTemplate(true);
		setMsg(null);
		setErr(null);
		setWarnings([]);
		rerender();

		fetch("/api/config/template")
			.then((r) => {
				if (!r.ok) {
					return { error: `HTTP ${r.status}: Failed to load template` };
				}
				return r.json().catch(() => ({ error: "Invalid JSON response" }));
			})
			.then((d: { error?: string; toml?: string }) => {
				setResettingTemplate(false);
				if (d.error) {
					setErr(d.error);
				} else {
					setToml(d.toml || "");
					setMsg("Loaded default template with all options. Review and save when ready.");
				}
				rerender();
			})
			.catch((fetchErr: Error) => {
				setResettingTemplate(false);
				let errMsg = fetchErr.message || "Network error";
				if (errMsg.includes("pattern")) {
					errMsg = "Failed to connect to server";
				}
				setErr(errMsg);
				rerender();
			});
	}

	if (configLoading) {
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<h2 className="text-lg font-medium text-[var(--text-strong)]">Configuration</h2>
				<Loading />
			</div>
		);
	}

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<h2 className="text-lg font-medium text-[var(--text-strong)]">Configuration</h2>
			<p className="text-xs text-[var(--muted)] leading-relaxed" style={{ maxWidth: "700px", margin: 0 }}>
				Edit the full moltis configuration. This includes server, tools, LLM providers, auth, and all other settings.
				Test your changes before saving. Changes require a restart to take effect.{" "}
				<a
					href="https://docs.moltis.org/configuration.html"
					target="_blank"
					rel="noopener"
					style={{ color: "var(--accent)", textDecoration: "underline" }}
				>
					View documentation {"\u2197"}
				</a>
			</p>
			{configPath ? (
				<div className="text-xs text-[var(--muted)]" style={{ fontFamily: "var(--font-mono)" }}>
					<span style={{ opacity: 0.7 }}>File:</span> {configPath}
				</div>
			) : null}

			<form onSubmit={onSave} style={{ maxWidth: "800px" }}>
				<div style={{ marginBottom: "12px" }}>
					<textarea
						className="provider-key-input"
						rows={20}
						style={{
							width: "100%",
							minHeight: "320px",
							resize: "vertical",
							fontFamily: "var(--font-mono)",
							fontSize: ".78rem",
							lineHeight: 1.5,
							whiteSpace: "pre",
							overflowWrap: "normal",
							overflowX: "auto",
						}}
						value={toml}
						onInput={(e: Event) => {
							setToml(targetValue(e));
							setMsg(null);
							setErr(null);
							setWarnings([]);
						}}
						spellcheck={false}
					/>
				</div>

				{warnings.length > 0 ? (
					<div
						style={{
							marginBottom: "12px",
							padding: "10px 12px",
							background: "color-mix(in srgb, orange 10%, transparent)",
							border: "1px solid orange",
							borderRadius: "6px",
						}}
					>
						<div className="text-xs font-medium" style={{ color: "orange", marginBottom: "6px" }}>
							Warnings:
						</div>
						<ul style={{ margin: 0, paddingLeft: "16px" }}>
							{warnings.map((w, i) => (
								<li key={i} className="text-xs text-[var(--muted)]" style={{ margin: "4px 0" }}>
									{w}
								</li>
							))}
						</ul>
					</div>
				) : null}

				<div style={{ display: "flex", alignItems: "center", gap: "8px", flexWrap: "wrap" }}>
					<button
						type="button"
						className="provider-btn provider-btn-secondary"
						onClick={onTest}
						disabled={testing || saving || resettingTemplate || restarting}
					>
						{testing ? "Testing\u2026" : "Test"}
					</button>
					<button
						type="button"
						className="provider-btn provider-btn-secondary"
						onClick={onReset}
						disabled={saving || testing || resettingTemplate || restarting}
					>
						Reload
					</button>
					<button
						type="button"
						className="provider-btn provider-btn-secondary"
						onClick={onResetToTemplate}
						disabled={saving || testing || resettingTemplate || restarting}
					>
						{resettingTemplate ? "Resetting\u2026" : "Reset to defaults"}
					</button>
					<button
						type="button"
						className="provider-btn provider-btn-danger"
						onClick={onRestart}
						disabled={saving || testing || resettingTemplate || restarting}
					>
						{restarting ? "Restarting\u2026" : "Restart"}
					</button>
					<div style={{ flex: 1 }} />
					<button
						type="submit"
						className="provider-btn"
						disabled={saving || testing || resettingTemplate || restarting}
					>
						{saving ? "Saving\u2026" : "Save"}
					</button>
				</div>

				{msg ? (
					<div className="text-xs" style={{ marginTop: "8px", color: "var(--accent)" }}>
						{msg}
					</div>
				) : null}
				{err ? (
					<div
						className="text-xs"
						style={{ marginTop: "8px", color: "var(--error)", whiteSpace: "pre-wrap", fontFamily: "var(--font-mono)" }}
					>
						{err}
					</div>
				) : null}
				{restarting ? (
					<div className="text-xs text-[var(--muted)]" style={{ marginTop: "8px" }}>
						The page will reload automatically when the server is back up.
					</div>
				) : null}
			</form>

			<div style={{ maxWidth: "800px", marginTop: "8px", paddingTop: "16px", borderTop: "1px solid var(--border)" }}>
				<p className="text-xs text-[var(--muted)] leading-relaxed">
					<strong>Tip:</strong> Click "Load Template" to see all available configuration options with documentation.
					This replaces the editor content with a fully documented template - copy your current values first if needed.
				</p>
			</div>

			<DeleteAllSessionsCard />
		</div>
	);
}

function DeleteAllSessionsCard(): VNode {
	const [deleting, setDeleting] = useState(false);

	function onDelete(): void {
		if (deleting) return;
		setDeleting(true);
		rerender();
		clearAllSessions()
			.then((res) => {
				if (res?.ok && !res?.skipped) {
					showToast("All sessions deleted", "success");
				}
			})
			.finally(() => {
				setDeleting(false);
				rerender();
			});
	}

	return (
		<div
			style={{
				maxWidth: "800px",
				marginTop: "16px",
				paddingTop: "16px",
				borderTop: "1px solid var(--border)",
			}}
		>
			<div className="text-sm font-medium text-[var(--text-strong)]" style={{ marginBottom: "6px" }}>
				Danger zone
			</div>
			<p className="text-xs text-[var(--muted)] leading-relaxed" style={{ margin: "0 0 10px" }}>
				Permanently delete all chat sessions except the main session. This cannot be undone.
			</p>
			<button
				type="button"
				className="provider-btn provider-btn-sm inline-flex items-center gap-1.5"
				style={{ background: "var(--error)", borderColor: "var(--error)", color: "#fff" }}
				onClick={onDelete}
				disabled={deleting}
			>
				<span className="icon icon-sm icon-x-circle shrink-0" />
				{deleting ? "Deleting\u2026" : "Delete all sessions"}
			</button>
		</div>
	);
}
