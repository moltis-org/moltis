// ── Tabbed import step for onboarding ────────────────────────
//
// Wraps all detected import sources (OpenClaw, Claude Code, Codex, Hermes)
// behind a tab bar, reusing the settings import section components.

import type { VNode } from "preact";
import { useEffect, useState } from "preact/hooks";
import { TabBar } from "../../components/forms";
import { get as getGon } from "../../gon";
import { sendRpc } from "../../helpers";
import { ClaudeImportSection } from "../../pages/sections/ClaudeImportSection";
import { CodexImportSection } from "../../pages/sections/CodexImportSection";
import { HermesImportSection } from "../../pages/sections/HermesImportSection";
import { OpenClawImportSection } from "../../pages/sections/OpenClawImportSection";
import { ensureWsConnected } from "../shared";

const WS_RETRY_LIMIT = 75;
const WS_RETRY_DELAY_MS = 200;

interface ImportTabDef {
	id: string;
	label: string;
	icon: VNode;
	detected: boolean;
	detectRpc: string;
	countFn: (payload: Record<string, unknown>) => number;
}

function countOpenClaw(p: Record<string, unknown>): number {
	let n = 0;
	if (p.identity_available) n++;
	if (p.providers_available) n++;
	n += Number(p.skills_count) || 0;
	if (p.memory_available) n++;
	if (p.channels_available) n++;
	n += Number(p.sessions_count) || 0;
	return n;
}

function countClaude(p: Record<string, unknown>): number {
	let n = 0;
	if (p.has_mcp_servers) n++;
	n += (Number(p.skills_count) || 0) + (Number(p.commands_count) || 0);
	if (p.has_memory) n++;
	return n;
}

function countCodex(p: Record<string, unknown>): number {
	let n = Number(p.mcp_servers_count) || 0;
	if (p.has_memory) n++;
	return n;
}

function countHermes(p: Record<string, unknown>): number {
	let n = Number(p.credentials_count) || 0;
	n += Number(p.skills_count) || 0;
	const memFiles = p.memory_files;
	if (Array.isArray(memFiles)) n += memFiles.length;
	else if (p.has_memory) n++;
	return n;
}

/** Build tab definitions at render time so getGon() reads current state. */
function getAllTabs(): ImportTabDef[] {
	return [
		{
			id: "openclaw",
			label: "OpenClaw",
			icon: <span className="icon icon-openclaw" />,
			detected: getGon("openclaw_detected") === true,
			detectRpc: "openclaw.scan",
			countFn: countOpenClaw,
		},
		{
			id: "claude",
			label: "Claude Code",
			icon: <span className="icon icon-terminal-cmd" />,
			detected: getGon("claude_detected") === true,
			detectRpc: "claude.detect",
			countFn: countClaude,
		},
		{
			id: "codex",
			label: "Codex CLI",
			icon: <span className="icon icon-code" />,
			detected: getGon("codex_detected") === true,
			detectRpc: "codex.detect",
			countFn: countCodex,
		},
		{
			id: "hermes",
			label: "Hermes",
			icon: <span className="icon icon-globe" />,
			detected: getGon("hermes_detected") === true,
			detectRpc: "hermes.detect",
			countFn: countHermes,
		},
	];
}

export function ImportStep({ onNext, onBack }: { onNext: () => void; onBack?: (() => void) | null }): VNode {
	const detectedTabs = getAllTabs().filter((t) => t.detected);
	const [activeTab, setActiveTab] = useState(detectedTabs[0]?.id || "openclaw");
	const [wsReady, setWsReady] = useState(false);
	const [badges, setBadges] = useState<Record<string, number>>({});

	// Ensure WS is connected with retry before showing import sections
	useEffect(() => {
		let cancelled = false;
		let attempts = 0;
		let timer: ReturnType<typeof setTimeout> | null = null;

		function tryConnect(): void {
			if (cancelled) return;
			ensureWsConnected();
			(sendRpc("openclaw.scan", {}) as Promise<{ ok?: boolean; error?: { code?: string; message?: string } }>).then(
				(res) => {
					if (cancelled) return;
					if (res?.ok || (res?.error?.code !== "UNAVAILABLE" && res?.error?.message !== "WebSocket not connected")) {
						setWsReady(true);
						return;
					}
					if (attempts < WS_RETRY_LIMIT) {
						attempts += 1;
						timer = setTimeout(tryConnect, WS_RETRY_DELAY_MS);
					} else {
						setWsReady(true);
					}
				},
			);
		}

		tryConnect();
		return () => {
			cancelled = true;
			if (timer) clearTimeout(timer);
		};
	}, []);

	// Fetch item counts for badges once WS is ready
	useEffect(() => {
		if (!wsReady) return;
		for (const tab of detectedTabs) {
			sendRpc(tab.detectRpc, {}).then((res: { ok?: boolean; payload?: unknown }) => {
				if (res?.ok && res.payload) {
					const count = tab.countFn(res.payload as Record<string, unknown>);
					if (count > 0) {
						setBadges((prev) => ({ ...prev, [tab.id]: count }));
					}
				}
			});
		}
	}, [wsReady]);

	if (!wsReady) {
		return (
			<div className="flex flex-col items-center justify-center gap-3 min-h-[200px]">
				<div className="inline-block w-8 h-8 border-2 border-[var(--border)] border-t-[var(--accent)] rounded-full animate-spin" />
				<div className="text-sm text-[var(--muted)]">Connecting&hellip;</div>
			</div>
		);
	}

	const tabs = detectedTabs.map((t) => ({
		id: t.id,
		label: t.label,
		icon: t.icon,
		badge: badges[t.id] as number | undefined,
	}));

	return (
		<div className="flex flex-col gap-4">
			<h2 className="text-lg font-medium text-[var(--text-strong)]">Import Your Data</h2>
			<p className="text-xs text-[var(--muted)] leading-relaxed">
				We detected existing installations you can import from. This is a read-only copy &mdash; your original files
				will not be modified. You can re-import at any time from Settings.
			</p>
			{tabs.length > 1 ? <TabBar tabs={tabs} active={activeTab} onChange={setActiveTab} /> : null}
			<div>{renderTab(activeTab)}</div>
			<div className="flex flex-wrap items-center gap-3 mt-1">
				{onBack ? (
					<button type="button" className="provider-btn provider-btn-secondary" onClick={onBack}>
						Back
					</button>
				) : null}
				<button type="button" className="provider-btn" onClick={onNext}>
					Continue
				</button>
				<button
					type="button"
					className="text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline"
					onClick={onNext}
				>
					Skip for now
				</button>
			</div>
		</div>
	);
}

function renderTab(id: string): VNode | null {
	switch (id) {
		case "openclaw":
			return <OpenClawImportSection />;
		case "claude":
			return <ClaudeImportSection />;
		case "codex":
			return <CodexImportSection />;
		case "hermes":
			return <HermesImportSection />;
		default:
			return null;
	}
}
