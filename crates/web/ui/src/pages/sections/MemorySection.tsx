// ── Memory section ────────────────────────────────────────────

import type { VNode } from "preact";
import { useEffect, useState } from "preact/hooks";
import { SaveButton, SectionHeading, StatusMessage, SubHeading, useSaveState } from "../../components/forms";
import { sendRpc } from "../../helpers";
import { targetChecked, targetValue } from "../../typed-events";
import type { RpcResponse } from "./_shared";
import { rerender } from "./_shared";

interface MemoryStatus {
	total_files?: number;
	total_chunks?: number;
	embedding_model?: string;
	db_size_display?: string;
}

interface MemoryConfig {
	style?: string;
	agent_write_mode?: string;
	user_profile_write_mode?: string;
	backend?: string;
	provider?: string;
	citations?: string;
	llm_reranking?: boolean;
	search_merge_strategy?: string;
	session_export?: string;
	prompt_memory_mode?: string;
	qmd_feature_enabled?: boolean;
	enable_prefetch?: boolean;
	prefetch_limit?: number;
	auto_extract_interval?: number;
	enable_session_summary?: boolean;
	enable_self_improvement?: boolean;
}

interface QmdStatus {
	available?: boolean;
	version?: string;
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Large component managing memory settings with QMD integration
export function MemorySection(): VNode {
	const [memStatus, setMemStatus] = useState<MemoryStatus | null>(null);
	const [memConfig, setMemConfig] = useState<MemoryConfig | null>(null);
	const [qmdStatus, setQmdStatus] = useState<QmdStatus | null>(null);
	const [memLoading, setMemLoading] = useState(true);
	const save = useSaveState();

	const [style, setStyle] = useState("hybrid");
	const [agentWriteMode, setAgentWriteMode] = useState("hybrid");
	const [userProfileWriteMode, setUserProfileWriteMode] = useState("explicit-and-auto");
	const [backend, setBackend] = useState("builtin");
	const [provider, setProvider] = useState("auto");
	const [citations, setCitations] = useState("auto");
	const [llmReranking, setLlmReranking] = useState(false);
	const [searchMergeStrategy, setSearchMergeStrategy] = useState("rrf");
	const [sessionExport, setSessionExport] = useState("on-new-or-reset");
	const [promptMemoryMode, setPromptMemoryMode] = useState("live-reload");
	const [enablePrefetch, setEnablePrefetch] = useState(true);
	const [prefetchLimit, setPrefetchLimit] = useState(3);
	const [autoExtractInterval, setAutoExtractInterval] = useState(5);
	const [enableSessionSummary, setEnableSessionSummary] = useState(true);
	const [enableSelfImprovement, setEnableSelfImprovement] = useState(true);

	useEffect(() => {
		Promise.all([sendRpc("memory.status", {}), sendRpc("memory.config.get", {}), sendRpc("memory.qmd.status", {})])
			.then(([statusRes, configRes, qmdRes]: [RpcResponse, RpcResponse, RpcResponse]) => {
				if (statusRes?.ok) {
					setMemStatus(statusRes.payload as MemoryStatus);
				}
				if (configRes?.ok) {
					const cfg = configRes.payload as MemoryConfig;
					setMemConfig(cfg);
					setStyle(cfg.style || "hybrid");
					setAgentWriteMode(cfg.agent_write_mode || "hybrid");
					setUserProfileWriteMode(cfg.user_profile_write_mode || "explicit-and-auto");
					setBackend(cfg.backend || "builtin");
					setProvider(cfg.provider || "auto");
					setCitations(cfg.citations || "auto");
					setLlmReranking(cfg.llm_reranking ?? false);
					setSearchMergeStrategy(cfg.search_merge_strategy || "rrf");
					setSessionExport(cfg.session_export || "on-new-or-reset");
					setPromptMemoryMode(cfg.prompt_memory_mode || "live-reload");
					setEnablePrefetch(cfg.enable_prefetch ?? true);
					setPrefetchLimit(cfg.prefetch_limit ?? 3);
					setAutoExtractInterval(cfg.auto_extract_interval ?? 5);
					setEnableSessionSummary(cfg.enable_session_summary ?? true);
					setEnableSelfImprovement(cfg.enable_self_improvement ?? true);
				}
				if (qmdRes?.ok) {
					setQmdStatus(qmdRes.payload as QmdStatus);
				}
				setMemLoading(false);
				rerender();
			})
			.catch(() => {
				setMemLoading(false);
				rerender();
			});
	}, []);

	function onSave(e: Event): void {
		e.preventDefault();
		save.setError(null);
		save.setSaving(true);

		sendRpc("memory.config.update", {
			style,
			agent_write_mode: agentWriteMode,
			user_profile_write_mode: userProfileWriteMode,
			backend,
			provider,
			citations,
			llm_reranking: llmReranking,
			search_merge_strategy: searchMergeStrategy,
			session_export: sessionExport,
			prompt_memory_mode: promptMemoryMode,
			enable_prefetch: enablePrefetch,
			prefetch_limit: prefetchLimit,
			auto_extract_interval: autoExtractInterval,
			enable_session_summary: enableSessionSummary,
			enable_self_improvement: enableSelfImprovement,
		}).then((res: RpcResponse) => {
			save.setSaving(false);
			if (res?.ok) {
				setMemConfig(res.payload as MemoryConfig);
				save.flashSaved();
			} else {
				save.setError((res?.error as { message?: string })?.message || "Failed to save");
			}
			rerender();
		});
	}

	if (memLoading) {
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<SectionHeading title="Memory" />
				<div className="text-xs text-[var(--muted)]">Loading{"\u2026"}</div>
			</div>
		);
	}

	const qmdFeatureEnabled = memConfig?.qmd_feature_enabled !== false;
	const qmdAvailable = qmdStatus?.available === true;

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<SectionHeading title="Memory" />
			<p className="text-xs text-[var(--muted)] leading-relaxed max-w-form" style={{ margin: 0 }}>
				Configure how the agent stores and retrieves long-term memory. Memory enables the agent to recall past
				conversations, notes, and context across sessions.
			</p>

			{memStatus ? (
				<div
					style={{
						maxWidth: "600px",
						padding: "12px 16px",
						borderRadius: "6px",
						border: "1px solid var(--border)",
						background: "var(--bg)",
					}}
				>
					<SubHeading title="Status" />
					<div style={{ display: "grid", gridTemplateColumns: "repeat(2,1fr)", gap: "8px 16px", fontSize: ".8rem" }}>
						<div>
							<span className="text-[var(--muted)]">Files:</span>
							<span className="text-[var(--text)]" style={{ marginLeft: "6px" }}>
								{memStatus.total_files || 0}
							</span>
						</div>
						<div>
							<span className="text-[var(--muted)]">Chunks:</span>
							<span className="text-[var(--text)]" style={{ marginLeft: "6px" }}>
								{memStatus.total_chunks || 0}
							</span>
						</div>
						<div>
							<span className="text-[var(--muted)]">Model:</span>
							<span
								className="text-[var(--text)]"
								style={{ marginLeft: "6px", fontFamily: "var(--font-mono)", fontSize: ".75rem" }}
							>
								{memStatus.embedding_model || "none"}
							</span>
						</div>
						<div>
							<span className="text-[var(--muted)]">DB Size:</span>
							<span className="text-[var(--text)]" style={{ marginLeft: "6px" }}>
								{memStatus.db_size_display || "0 B"}
							</span>
						</div>
					</div>
				</div>
			) : null}

			<form onSubmit={onSave} style={{ maxWidth: "600px", display: "flex", flexDirection: "column", gap: "16px" }}>
				<div>
					<SubHeading title="Memory Style" />
					<p className="text-xs text-[var(--muted)]" style={{ margin: "0 0 8px" }}>
						Choose the high-level orchestration model. This controls whether prompt-visible <code>MEMORY.md</code> and
						memory tools are both active, one is active, or both are off.
					</p>
					<select
						className="provider-key-input"
						style={{ width: "auto", minWidth: "240px" }}
						value={style}
						onChange={(e: Event) => {
							setStyle(targetValue(e));
							rerender();
						}}
					>
						<option value="hybrid">Hybrid</option>
						<option value="prompt-only">Prompt-only</option>
						<option value="search-only">Search-only</option>
						<option value="off">Off</option>
					</select>
				</div>

				<div>
					<SubHeading title="Backend" />
					<div
						style={{
							marginBottom: "12px",
							padding: "12px",
							borderRadius: "6px",
							border: "1px solid var(--border)",
							background: "var(--bg)",
							fontSize: ".75rem",
						}}
					>
						<table style={{ width: "100%", borderCollapse: "collapse" }}>
							<thead>
								<tr style={{ borderBottom: "1px solid var(--border)" }}>
									<th style={{ textAlign: "left", padding: "4px 8px 8px 0", color: "var(--muted)", fontWeight: 500 }}>
										Feature
									</th>
									<th style={{ textAlign: "center", padding: "4px 8px 8px", color: "var(--muted)", fontWeight: 500 }}>
										Built-in
									</th>
									<th style={{ textAlign: "center", padding: "4px 8px 8px", color: "var(--muted)", fontWeight: 500 }}>
										QMD
									</th>
								</tr>
							</thead>
							<tbody>
								<tr>
									<td style={{ padding: "6px 8px 6px 0", color: "var(--text)" }}>Search type</td>
									<td style={{ padding: "6px 8px", textAlign: "center", color: "var(--muted)" }}>FTS5 + vector</td>
									<td style={{ padding: "6px 8px", textAlign: "center", color: "var(--muted)" }}>
										BM25 + vector + LLM
									</td>
								</tr>
								<tr>
									<td style={{ padding: "6px 8px 6px 0", color: "var(--text)" }}>External dependency</td>
									<td style={{ padding: "6px 8px", textAlign: "center", color: "var(--accent)" }}>None</td>
									<td style={{ padding: "6px 8px", textAlign: "center", color: "var(--muted)" }}>Node.js/Bun</td>
								</tr>
								<tr>
									<td style={{ padding: "6px 8px 6px 0", color: "var(--text)" }}>Embedding cache</td>
									<td style={{ padding: "6px 8px", textAlign: "center", color: "var(--accent)" }}>{"\u2713"}</td>
									<td style={{ padding: "6px 8px", textAlign: "center", color: "var(--muted)" }}>{"\u2717"}</td>
								</tr>
								<tr>
									<td style={{ padding: "6px 8px 6px 0", color: "var(--text)" }}>OpenAI batch API</td>
									<td style={{ padding: "6px 8px", textAlign: "center", color: "var(--accent)" }}>
										{"\u2713"} (50% cheaper)
									</td>
									<td style={{ padding: "6px 8px", textAlign: "center", color: "var(--muted)" }}>{"\u2717"}</td>
								</tr>
								<tr>
									<td style={{ padding: "6px 8px 6px 0", color: "var(--text)" }}>Provider fallback</td>
									<td style={{ padding: "6px 8px", textAlign: "center", color: "var(--accent)" }}>{"\u2713"}</td>
									<td style={{ padding: "6px 8px", textAlign: "center", color: "var(--muted)" }}>{"\u2717"}</td>
								</tr>
								<tr>
									<td style={{ padding: "6px 8px 6px 0", color: "var(--text)" }}>LLM reranking</td>
									<td style={{ padding: "6px 8px", textAlign: "center", color: "var(--muted)" }}>Optional</td>
									<td style={{ padding: "6px 8px", textAlign: "center", color: "var(--accent)" }}>Built-in</td>
								</tr>
								<tr>
									<td style={{ padding: "6px 8px 6px 0", color: "var(--text)" }}>Best for</td>
									<td style={{ padding: "6px 8px", textAlign: "center", color: "var(--muted)" }}>Most users</td>
									<td style={{ padding: "6px 8px", textAlign: "center", color: "var(--muted)" }}>Power users</td>
								</tr>
							</tbody>
						</table>
					</div>

					<div style={{ display: "flex", gap: "8px" }}>
						<button
							type="button"
							className={`provider-btn ${backend === "builtin" ? "" : "provider-btn-secondary"}`}
							onClick={() => {
								setBackend("builtin");
								rerender();
							}}
						>
							Built-in (Recommended)
						</button>
						<button
							type="button"
							className={`provider-btn ${backend === "qmd" ? "" : "provider-btn-secondary"}`}
							disabled={!qmdFeatureEnabled}
							onClick={() => {
								setBackend("qmd");
								rerender();
							}}
						>
							QMD
						</button>
					</div>

					{qmdFeatureEnabled ? null : (
						<div className="text-xs text-[var(--error)]" style={{ marginTop: "8px" }}>
							QMD feature is not enabled. Rebuild moltis with{" "}
							<code style={{ fontFamily: "var(--font-mono)", fontSize: ".7rem" }}>--features qmd</code>
						</div>
					)}

					{backend === "qmd" ? (
						<div
							style={{
								marginTop: "12px",
								padding: "12px",
								borderRadius: "6px",
								border: "1px solid var(--border)",
								background: "var(--bg)",
							}}
						>
							<h4 className="text-xs font-medium text-[var(--text-strong)]" style={{ margin: "0 0 8px" }}>
								QMD Status
							</h4>
							{qmdAvailable ? (
								<div
									className="text-xs"
									style={{ color: "var(--accent)", display: "flex", alignItems: "center", gap: "6px" }}
								>
									<span>{"\u2713"}</span> QMD is installed{" "}
									{qmdStatus?.version ? <span className="text-[var(--muted)]">({qmdStatus.version})</span> : null}
								</div>
							) : (
								<div>
									<div className="text-xs" style={{ color: "var(--error)", marginBottom: "8px" }}>
										{"\u2717"} QMD is not installed or not found in PATH
									</div>
									<div className="text-xs text-[var(--muted)]" style={{ lineHeight: 1.6 }}>
										<strong style={{ color: "var(--text)" }}>Installation:</strong>
										<br />
										<code
											style={{
												fontFamily: "var(--font-mono)",
												fontSize: ".7rem",
												background: "var(--surface)",
												padding: "2px 4px",
												borderRadius: "3px",
											}}
										>
											npm install -g @tobilu/qmd
										</code>
										<span style={{ margin: "0 4px" }}>or</span>
										<code
											style={{
												fontFamily: "var(--font-mono)",
												fontSize: ".7rem",
												background: "var(--surface)",
												padding: "2px 4px",
												borderRadius: "3px",
											}}
										>
											bun install -g @tobilu/qmd
										</code>
										<br />
										<br />
										Verify the CLI is available:
										<code
											style={{
												display: "block",
												marginTop: "4px",
												fontFamily: "var(--font-mono)",
												fontSize: ".7rem",
												background: "var(--surface)",
												padding: "2px 4px",
												borderRadius: "3px",
											}}
										>
											qmd --version
										</code>
										<br />
										<a
											href="https://github.com/tobi/qmd"
											target="_blank"
											rel="noopener"
											style={{ color: "var(--accent)" }}
										>
											View documentation {"\u2192"}
										</a>
									</div>
								</div>
							)}
						</div>
					) : null}
				</div>

				<div>
					<SubHeading title="Prompt Memory Mode" />
					<p className="text-xs text-[var(--muted)]" style={{ margin: "0 0 8px" }}>
						When prompt memory is enabled, choose whether <code>MEMORY.md</code> is reread on every turn or frozen when
						the session starts.
					</p>
					<select
						className="provider-key-input"
						style={{ width: "auto", minWidth: "260px" }}
						value={promptMemoryMode}
						disabled={style === "search-only" || style === "off"}
						onChange={(e: Event) => {
							setPromptMemoryMode(targetValue(e));
							rerender();
						}}
					>
						<option value="live-reload">Live reload</option>
						<option value="frozen-at-session-start">Frozen at session start</option>
					</select>
					{style === "search-only" || style === "off" ? (
						<div className="text-xs text-[var(--muted)]" style={{ marginTop: "8px" }}>
							Prompt memory is disabled by the current memory style, so this setting will only matter after you
							re-enable prompt memory.
						</div>
					) : null}
				</div>

				<div>
					<SubHeading title="Agent Memory Writes" />
					<p className="text-xs text-[var(--muted)]" style={{ margin: "0 0 8px" }}>
						Control where agent-authored memory writes can land. This affects <code>memory_save</code> and silent
						compaction memory flushes.
					</p>
					<select
						className="provider-key-input"
						style={{ width: "auto", minWidth: "220px" }}
						value={agentWriteMode}
						onChange={(e: Event) => {
							setAgentWriteMode(targetValue(e));
							rerender();
						}}
					>
						<option value="hybrid">Hybrid (MEMORY.md and memory/*.md)</option>
						<option value="prompt-only">Prompt-only (MEMORY.md only)</option>
						<option value="search-only">Search-only (memory/*.md only)</option>
						<option value="off">Off</option>
					</select>
				</div>

				<div>
					<SubHeading title="USER.md Writes" />
					<p className="text-xs text-[var(--muted)]" style={{ margin: "0 0 8px" }}>
						Control whether Moltis mirrors your profile into <code>USER.md</code>, and whether browser or channel
						timezone/location signals can update it silently.
					</p>
					<select
						className="provider-key-input"
						style={{ width: "auto", minWidth: "250px" }}
						value={userProfileWriteMode}
						onChange={(e: Event) => {
							setUserProfileWriteMode(targetValue(e));
							rerender();
						}}
					>
						<option value="explicit-and-auto">Explicit and auto</option>
						<option value="explicit-only">Explicit only</option>
						<option value="off">Off (moltis.toml only)</option>
					</select>
				</div>

				<div>
					<SubHeading title="Embedding Provider" />
					<p className="text-xs text-[var(--muted)]" style={{ margin: "0 0 8px" }}>
						Select which embedding provider the built-in memory backend should use for RAG. QMD manages retrieval
						separately, so this setting is ignored while the QMD backend is active.
					</p>
					<select
						className="provider-key-input"
						style={{ width: "auto", minWidth: "220px" }}
						value={provider}
						disabled={backend === "qmd"}
						onChange={(e: Event) => {
							setProvider(targetValue(e));
							rerender();
						}}
					>
						<option value="auto">Auto-detect</option>
						<option value="local">Local GGUF</option>
						<option value="ollama">Ollama</option>
						<option value="openai">OpenAI</option>
						<option value="custom">Custom OpenAI-compatible</option>
					</select>
					{backend === "qmd" ? (
						<div className="text-xs text-[var(--muted)]" style={{ marginTop: "8px" }}>
							This setting is kept for when you switch back to the built-in backend.
						</div>
					) : null}
				</div>

				<div>
					<SubHeading title="Citations" />
					<p className="text-xs text-[var(--muted)]" style={{ margin: "0 0 8px" }}>
						Include source file and line number with search results to help track where information comes from.
					</p>
					<select
						className="provider-key-input"
						style={{ width: "auto", minWidth: "150px" }}
						value={citations}
						onChange={(e: Event) => {
							setCitations(targetValue(e));
							rerender();
						}}
					>
						<option value="auto">Auto (multi-file only)</option>
						<option value="on">Always</option>
						<option value="off">Never</option>
					</select>
				</div>

				<div>
					<SubHeading title="Search Merge Strategy" />
					<p className="text-xs text-[var(--muted)]" style={{ margin: "0 0 8px" }}>
						Choose how Moltis blends vector and keyword memory hits before optional reranking.
					</p>
					<select
						className="provider-key-input"
						style={{ width: "auto", minWidth: "180px" }}
						value={searchMergeStrategy}
						onChange={(e: Event) => {
							setSearchMergeStrategy(targetValue(e));
							rerender();
						}}
					>
						<option value="rrf">RRF</option>
						<option value="linear">Linear</option>
					</select>
				</div>

				<div>
					<label style={{ display: "flex", alignItems: "center", gap: "8px", cursor: "pointer" }}>
						<input
							type="checkbox"
							checked={llmReranking}
							onChange={(e: Event) => {
								setLlmReranking(targetChecked(e));
								rerender();
							}}
						/>
						<div>
							<span className="text-sm font-medium text-[var(--text-strong)]">LLM Reranking</span>
							<p className="text-xs text-[var(--muted)]" style={{ margin: "2px 0 0" }}>
								Use the LLM to rerank search results for better relevance (slower but more accurate).
							</p>
						</div>
					</label>
				</div>

				<div>
					<SubHeading title="Agent Self-Improvement" />
					<p className="text-xs text-[var(--muted)]" style={{ margin: "0 0 8px" }}>
						Controls how the agent learns autonomously across sessions.
					</p>
					<div style={{ display: "flex", flexDirection: "column", gap: "10px" }}>
						<label className="text-xs flex items-center gap-2 cursor-pointer">
							<input
								type="checkbox"
								checked={enableSelfImprovement}
								onChange={(e: Event) => {
									setEnableSelfImprovement(targetChecked(e));
									rerender();
								}}
							/>
							<div>
								<span className="text-[var(--text)]">Skill self-improvement prompting</span>
								<span className="text-[var(--muted)] block text-[.7rem]">
									Encourage the agent to create reusable skills after complex tasks
								</span>
							</div>
						</label>
						<label className="text-xs flex items-center gap-2 cursor-pointer">
							<input
								type="checkbox"
								checked={enablePrefetch}
								onChange={(e: Event) => {
									setEnablePrefetch(targetChecked(e));
									rerender();
								}}
							/>
							<div>
								<span className="text-[var(--text)]">Memory recall (prefetch)</span>
								<span className="text-[var(--muted)] block text-[.7rem]">
									Automatically recall relevant memories before each turn
								</span>
							</div>
						</label>
						{enablePrefetch ? (
							<div style={{ marginLeft: "24px" }}>
								<label className="text-xs text-[var(--muted)]">
									Max results per turn:{" "}
									<input
										type="number"
										min={1}
										max={10}
										className="provider-key-input"
										style={{ width: "60px", marginLeft: "4px" }}
										value={prefetchLimit}
										onChange={(e: Event) => {
											setPrefetchLimit(Number.parseInt(targetValue(e), 10) || 3);
											rerender();
										}}
									/>
								</label>
							</div>
						) : null}
						<label className="text-xs flex items-center gap-2 cursor-pointer">
							<input
								type="checkbox"
								checked={autoExtractInterval > 0}
								onChange={(e: Event) => {
									setAutoExtractInterval(targetChecked(e) ? 5 : 0);
									rerender();
								}}
							/>
							<div>
								<span className="text-[var(--text)]">Periodic memory extraction</span>
								<span className="text-[var(--muted)] block text-[.7rem]">
									Automatically save important context every N turns
								</span>
							</div>
						</label>
						{autoExtractInterval > 0 ? (
							<div style={{ marginLeft: "24px" }}>
								<label className="text-xs text-[var(--muted)]">
									Every{" "}
									<input
										type="number"
										min={1}
										max={50}
										className="provider-key-input"
										style={{ width: "60px", margin: "0 4px" }}
										value={autoExtractInterval}
										onChange={(e: Event) => {
											setAutoExtractInterval(Number.parseInt(targetValue(e), 10) || 5);
											rerender();
										}}
									/>
									turns
								</label>
							</div>
						) : null}
						<label className="text-xs flex items-center gap-2 cursor-pointer">
							<input
								type="checkbox"
								checked={enableSessionSummary}
								onChange={(e: Event) => {
									setEnableSessionSummary(targetChecked(e));
									rerender();
								}}
							/>
							<div>
								<span className="text-[var(--text)]">Session-end summary</span>
								<span className="text-[var(--muted)] block text-[.7rem]">
									Summarize accomplishments when a session is reset
								</span>
							</div>
						</label>
					</div>
				</div>

				<div>
					<SubHeading title="Session Export" />
					<p className="text-xs text-[var(--muted)]" style={{ margin: "0 0 8px" }}>
						Export session transcripts into searchable memory when a session is rolled over.
					</p>
					<select
						className="provider-key-input"
						style={{ width: "auto", minWidth: "220px" }}
						value={sessionExport}
						onChange={(e: Event) => {
							setSessionExport(targetValue(e));
							rerender();
						}}
					>
						<option value="on-new-or-reset">On /new and /reset</option>
						<option value="off">Off</option>
					</select>
				</div>

				<div
					style={{
						display: "flex",
						alignItems: "center",
						gap: "8px",
						paddingTop: "8px",
						borderTop: "1px solid var(--border)",
					}}
				>
					<SaveButton saving={save.saving} saved={save.saved} type="submit" />
					<StatusMessage error={save.error} success={save.saved ? "Saved" : null} />
				</div>
			</form>
		</div>
	);
}
