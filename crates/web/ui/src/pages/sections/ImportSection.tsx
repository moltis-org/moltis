// ── Imports section — tabs for each detected import source ────

import type { VNode } from "preact";
import { useState } from "preact/hooks";
import { TabBar } from "../../components/forms";
import * as gon from "../../gon";
import { ClaudeImportSection } from "./ClaudeImportSection";
import { HermesImportSection } from "./HermesImportSection";
import { OpenClawImportSection } from "./OpenClawImportSection";

interface ImportTab {
	id: string;
	label: string;
	detected: boolean;
}

const ALL_TABS: ImportTab[] = [
	{ id: "openclaw", label: "OpenClaw", detected: gon.get("openclaw_detected") === true },
	{ id: "claude", label: "Claude Code", detected: gon.get("claude_detected") === true },
	{ id: "hermes", label: "Hermes", detected: gon.get("hermes_detected") === true },
];

export function ImportSection(): VNode {
	const tabs = ALL_TABS.filter((t) => t.detected);
	const [activeTab, setActiveTab] = useState(tabs[0]?.id || "openclaw");

	// Single source detected — render directly without tab bar
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
		case "openclaw":
			return <OpenClawImportSection />;
		case "claude":
			return <ClaudeImportSection />;
		case "hermes":
			return <HermesImportSection />;
		default:
			return null;
	}
}
