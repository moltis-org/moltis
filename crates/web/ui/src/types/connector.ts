export type ConnectorKind = "caldav";

export interface ConnectorDescriptor {
	kind: ConnectorKind;
	displayName: string;
}

export interface ConnectorAccount {
	id: string;
	kind: ConnectorKind;
	name: string;
	serverUrl: string;
	username: string;
	timeoutSeconds: number;
	allowInsecureHttp: boolean;
	allowPrivateNetwork: boolean;
	hasPassword: boolean;
	managed: boolean;
	enabled: boolean;
	createdAt: string;
	updatedAt: string;
}

export interface ConnectorCalendar {
	href: string;
	displayName?: string;
	color?: string;
	description?: string;
	collectionEtag?: string;
	supportsSync: boolean;
}

export type CalendarSelection = { mode: "all" } | { mode: "selected"; calendarHrefs: string[] };

export interface ConnectorDatasetFilters {
	startDate?: string | null;
	endDate?: string | null;
	acceptedByAccount: boolean;
}

export interface ConnectorDatasetConfig {
	schemaVersion: number;
	selection: CalendarSelection;
	filters: ConnectorDatasetFilters;
}

export interface ConnectorProjections {
	jsonl: boolean;
	markdown: boolean;
}

export interface ConnectorDataset {
	id: string;
	accountId: string;
	name: string;
	instruction?: string;
	config: ConnectorDatasetConfig;
	scheduleMinutes?: number | null;
	projections: ConnectorProjections;
	enabled: boolean;
	lastSyncAt?: string;
	nextSyncAt?: string;
	lastError?: string;
	itemCount: number;
	projectionPath?: string;
	needsSync: boolean;
	createdAt: string;
	updatedAt: string;
}

export interface ConnectorDatasetDraft {
	name: string;
	config: ConnectorDatasetConfig;
	scheduleMinutes?: number | null;
	projections: ConnectorProjections;
	enabled: boolean;
}

export interface ConnectorDatasetCompileResponse {
	draft: ConnectorDatasetDraft;
	summary: string;
	warnings: string[];
}

export type ConnectorRunStatus = "running" | "succeeded" | "failed";

export interface ConnectorRun {
	id: string;
	datasetId: string;
	status: ConnectorRunStatus;
	startedAt: string;
	finishedAt?: string;
	upserted: number;
	deleted: number;
	active: number;
	error?: string;
}

export type JsonValue = string | number | boolean | null | JsonValue[] | { [key: string]: JsonValue };

export interface ConnectorItem {
	id: string;
	datasetId: string;
	remoteId: string;
	kind: string;
	remoteVersion?: string;
	occurredAt?: string;
	updatedAt?: string;
	bodyJson: JsonValue;
	contentHash: string;
	createdAt: string;
	storedAt: string;
	deletedAt?: string;
}

export interface AvailableConnectorsResponse {
	connectors: ConnectorDescriptor[];
}

export interface ConnectorAccountsResponse {
	accounts: ConnectorAccount[];
}

export interface ConnectorCalendarsResponse {
	calendars: ConnectorCalendar[];
}

export interface ConnectorDatasetsResponse {
	datasets: ConnectorDataset[];
}

export interface ConnectorRunsResponse {
	runs: ConnectorRun[];
}

export interface ConnectorItemsResponse {
	items: ConnectorItem[];
}

export interface ConnectorRemovedResponse {
	removed: boolean;
}
