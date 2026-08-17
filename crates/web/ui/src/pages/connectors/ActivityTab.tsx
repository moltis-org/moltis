import type { VNode } from "preact";
import { useEffect, useState } from "preact/hooks";
import { Badge, EmptyState, Loading, SelectField, SettingsCard, StatusMessage } from "../../components/forms";
import { useTranslation } from "../../i18n";
import type { ConnectorDataset, ConnectorRun, ConnectorRunStatus } from "../../types/connector";
import { connectorRpc } from "./rpc";

const ACTIVITY_LIMIT = 30;
const POLL_INTERVAL_MS = 20_000;

interface ActivityTabProps {
	datasets: ConnectorDataset[];
}

function formatTimestamp(value?: string): string {
	if (!value) return "";
	const date = new Date(value);
	return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

function statusVariant(status: ConnectorRunStatus): "running" | "configured" | "error" {
	if (status === "running") return "running";
	if (status === "succeeded") return "configured";
	return "error";
}

export function ActivityTab({ datasets }: ActivityTabProps): VNode {
	const { t } = useTranslation("connectors");
	const [selectedId, setSelectedId] = useState(datasets[0]?.id ?? "");
	const [runs, setRuns] = useState<ConnectorRun[]>([]);
	const [loading, setLoading] = useState(Boolean(selectedId));
	const [error, setError] = useState<string | null>(null);
	const [refreshVersion, setRefreshVersion] = useState(0);

	useEffect(() => {
		if (selectedId && datasets.some((dataset) => dataset.id === selectedId)) return;
		setSelectedId(datasets[0]?.id ?? "");
	}, [datasets, selectedId]);

	useEffect(() => {
		if (!selectedId) {
			setRuns([]);
			setLoading(false);
			setError(null);
			return;
		}
		let active = true;
		async function load(showLoading: boolean): Promise<void> {
			if (showLoading) setLoading(true);
			try {
				const payload = await connectorRpc("connectors.runs.list", { datasetId: selectedId, limit: ACTIVITY_LIMIT });
				if (!active) return;
				setRuns(payload.runs);
				setError(null);
			} catch (caught: unknown) {
				if (active) setError(caught instanceof Error ? caught.message : String(caught));
			} finally {
				if (active) setLoading(false);
			}
		}
		void load(true);
		const interval = window.setInterval(() => void load(false), POLL_INTERVAL_MS);
		return () => {
			active = false;
			window.clearInterval(interval);
		};
	}, [selectedId, refreshVersion]);

	if (datasets.length === 0) return <EmptyState message={t("activity.emptyDatasets")} />;

	return (
		<div className="flex flex-col gap-3">
			<div className="flex flex-col gap-2 sm:flex-row sm:items-end">
				<SelectField
					id="connector-activity-dataset"
					label={t("activity.dataset")}
					value={selectedId}
					onChange={setSelectedId}
					className="min-w-0 flex-1 sm:max-w-sm"
					options={datasets.map((dataset) => ({ value: dataset.id, label: dataset.name }))}
				/>
				<button
					type="button"
					className="provider-btn provider-btn-secondary mb-3"
					disabled={loading}
					onClick={() => setRefreshVersion((value) => value + 1)}
				>
					{t("activity.refresh")}
				</button>
			</div>
			{loading ? <Loading message={t("activity.loading")} /> : null}
			<StatusMessage error={error} />
			{!(loading || error) && runs.length === 0 ? <EmptyState message={t("activity.emptyRuns")} /> : null}
			{runs.map((run) => (
				<SettingsCard key={run.id} className="bg-[var(--surface)] border border-[var(--border)] rounded-lg p-4">
					<div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
						<div>
							<div className="flex items-center gap-2">
								<Badge label={t(`status.${run.status}`)} variant={statusVariant(run.status)} />
								<span className="font-mono text-xs text-[var(--muted)]">{run.id}</span>
							</div>
							<div className="mt-2 text-xs text-[var(--muted)]">
								{t("activity.started", { time: formatTimestamp(run.startedAt) })}
								{run.finishedAt ? ` · ${t("activity.finished", { time: formatTimestamp(run.finishedAt) })}` : ""}
							</div>
							<div className="mt-1 text-xs text-[var(--muted)]">
								{t("activity.counts", { upserted: run.upserted, deleted: run.deleted, active: run.active })}
							</div>
							{run.error ? <StatusMessage error={run.error} className="mt-2 text-xs" /> : null}
						</div>
					</div>
				</SettingsCard>
			))}
		</div>
	);
}
