import type { VNode } from "preact";
import { render } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import { Loading, SectionHeading, StatusMessage, TabBar } from "../components/forms";
import { useTranslation } from "../i18n";
import { settingsPath } from "../routes";
import type {
	ConnectorAccount,
	ConnectorChannelSource,
	ConnectorDataset,
	ConnectorDescriptor,
} from "../types/connector";
import { ActivityTab } from "./connectors/ActivityTab";
import { ConnectionsTab } from "./connectors/ConnectionsTab";
import { DatasetsTab } from "./connectors/DatasetsTab";
import { connectorRpc } from "./connectors/rpc";

type ConnectorsTab = "connections" | "datasets" | "activity";

const VALID_TABS = new Set<ConnectorsTab>(["connections", "datasets", "activity"]);

let connectorsContainer: HTMLElement | null = null;

interface ConnectorsPageProps {
	initialTab: ConnectorsTab;
}

function parseTab(subPath?: string | null): ConnectorsTab {
	return VALID_TABS.has(subPath as ConnectorsTab) ? (subPath as ConnectorsTab) : "connections";
}

function ConnectorsPage({ initialTab }: ConnectorsPageProps): VNode {
	const { t } = useTranslation("connectors");
	const [activeTab, setActiveTab] = useState<ConnectorsTab>(initialTab);
	const [available, setAvailable] = useState<ConnectorDescriptor[]>([]);
	const [accounts, setAccounts] = useState<ConnectorAccount[]>([]);
	const [channelSources, setChannelSources] = useState<ConnectorChannelSource[]>([]);
	const [datasets, setDatasets] = useState<ConnectorDataset[]>([]);
	const [loading, setLoading] = useState(true);
	const [error, setError] = useState<string | null>(null);
	const loadRevision = useRef(0);

	async function load(propagate = false): Promise<void> {
		const revision = loadRevision.current + 1;
		loadRevision.current = revision;
		setLoading(true);
		try {
			const [availablePayload, accountsPayload, datasetsPayload, channelSourcesPayload] = await Promise.all([
				connectorRpc("connectors.available", {}),
				connectorRpc("connectors.accounts.list", {}),
				connectorRpc("connectors.datasets.list", {}),
				connectorRpc("connectors.channel_sources.list", {}),
			]);
			if (loadRevision.current === revision) {
				setAvailable(availablePayload.connectors);
				setAccounts(accountsPayload.accounts);
				setDatasets(datasetsPayload.datasets);
				setChannelSources(channelSourcesPayload.sources);
				setError(null);
			}
		} catch (caught: unknown) {
			if (loadRevision.current === revision) setError(caught instanceof Error ? caught.message : String(caught));
			if (propagate) throw caught;
		} finally {
			if (loadRevision.current === revision) setLoading(false);
		}
	}

	async function refreshAfterMutation(): Promise<void> {
		await load(true);
	}

	useEffect(() => {
		void load();
	}, []);

	function selectTab(id: string): void {
		if (!VALID_TABS.has(id as ConnectorsTab)) return;
		const next = id as ConnectorsTab;
		setActiveTab(next);
		history.replaceState(null, "", settingsPath(`connectors/${next}`));
	}

	return (
		<div className="flex-1 overflow-y-auto p-4 sm:p-6">
			<div className="mx-auto w-full max-w-5xl">
				<SectionHeading title={t("title")} subtitle={t("subtitle")}>
					<button
						type="button"
						className="provider-btn provider-btn-secondary"
						disabled={loading}
						onClick={() => void load()}
					>
						{t("refresh")}
					</button>
				</SectionHeading>
				<TabBar
					className="mb-4 flex overflow-x-auto border-b border-[var(--border)] text-xs"
					tabs={[
						{ id: "connections", label: t("tabs.connections"), badge: accounts.length },
						{ id: "datasets", label: t("tabs.datasets"), badge: datasets.length },
						{ id: "activity", label: t("tabs.activity") },
					]}
					active={activeTab}
					onChange={selectTab}
				/>
				{loading && accounts.length === 0 && datasets.length === 0 ? <Loading message={t("loading")} /> : null}
				<StatusMessage error={error} className="mb-3 text-xs" />
				{activeTab === "connections" ? (
					<ConnectionsTab
						accounts={accounts}
						caldavAvailable={available.some((connector) => connector.kind === "caldav")}
						channelHistoryAvailable={available.some((connector) => connector.kind === "channel_history")}
						gmailAvailable={available.some((connector) => connector.kind === "gmail")}
						himalayaAvailable={available.some((connector) => connector.kind === "himalaya")}
						channelSources={channelSources}
						onChanged={refreshAfterMutation}
					/>
				) : null}
				{activeTab === "datasets" ? (
					<DatasetsTab accounts={accounts} datasets={datasets} onChanged={refreshAfterMutation} />
				) : null}
				{activeTab === "activity" ? <ActivityTab datasets={datasets} /> : null}
			</div>
		</div>
	);
}

export function initConnectors(container: HTMLElement, subPath?: string | null): void {
	connectorsContainer = container;
	render(<ConnectorsPage initialTab={parseTab(subPath)} />, container);
}

export function teardownConnectors(): void {
	if (connectorsContainer) render(null, connectorsContainer);
	connectorsContainer = null;
}
