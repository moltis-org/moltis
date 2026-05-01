// ── Imports section — tabs for each detected import source ────

import type { VNode } from "preact";
import { useEffect, useState } from "preact/hooks";
import { TabBar } from "../../components/forms";
import * as gon from "../../gon";
import { sendRpc } from "../../helpers";
import type { RpcResponse } from "./_shared";
import { ClaudeImportSection } from "./ClaudeImportSection";
import { CodexImportSection } from "./CodexImportSection";
import { HermesImportSection } from "./HermesImportSection";
import { MoltisDataSection } from "./MoltisDataSection";
import { OpenClawImportSection } from "./OpenClawImportSection";

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

/** Build tab definitions at render time so gon.get() reads current state. */
function getAllTabs(): ImportTabDef[] {
	return [
		{
			id: "openclaw",
			label: "OpenClaw",
			icon: <span className="icon icon-openclaw" />,
			detected: gon.get("openclaw_detected") === true,
			detectRpc: "openclaw.scan",
			countFn: countOpenClaw,
		},
		{
			id: "claude",
			label: "Claude Code",
			icon: <span className="icon icon-terminal-cmd" />,
			detected: gon.get("claude_detected") === true,
			detectRpc: "claude.detect",
			countFn: countClaude,
		},
		{
			id: "codex",
			label: "Codex CLI",
			icon: <span className="icon icon-code" />,
			detected: gon.get("codex_detected") === true,
			detectRpc: "codex.detect",
			countFn: countCodex,
		},
		{
			id: "hermes",
			label: "Hermes",
			icon: <span className="icon icon-globe" />,
			detected: gon.get("hermes_detected") === true,
			detectRpc: "hermes.detect",
			countFn: countHermes,
		},
	];
}

export function ImportSection(): VNode {
	const detectedTabs = getAllTabs().filter((t) => t.detected);
	const [activeTab, setActiveTab] = useState("moltis");
	const [badges, setBadges] = useState<Record<string, number>>({});

	useEffect(() => {
		for (const tab of detectedTabs) {
			sendRpc(tab.detectRpc, {}).then((res: RpcResponse) => {
				if (res?.ok && res.payload) {
					const count = tab.countFn(res.payload as Record<string, unknown>);
					if (count > 0) {
						setBadges((prev) => ({ ...prev, [tab.id]: count }));
					}
				}
			});
		}
	}, []);

	// Moltis tab is always first, then detected external sources.
	const moltisTab = {
		id: "moltis",
		label: "Moltis",
		icon: <span className="icon icon-download" />,
		badge: undefined as number | undefined,
	};

	const externalTabs = detectedTabs.map((t) => ({
		id: t.id,
		label: t.label,
		icon: t.icon,
		badge: badges[t.id] as number | undefined,
	}));

	const tabs = [moltisTab, ...externalTabs];

	// Only Moltis tab — render directly without tab bar
	if (tabs.length === 1) {
		return <div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">{renderTab(tabs[0].id)}</div>;
	}

	return (
		<div className="flex-1 flex flex-col min-w-0 overflow-y-auto">
			<div className="px-4 pt-4">
				<TabBar tabs={tabs} active={activeTab} onChange={setActiveTab} />
			</div>
			<div className="p-4 flex flex-col gap-4">{renderTab(activeTab)}</div>
		</div>
	);
}

function renderTab(id: string): VNode | null {
	switch (id) {
		case "moltis":
			return <MoltisDataSection />;
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
