export type ConnectorKind = "caldav" | "channel_history" | "gmail" | "himalaya" | "tesla";

export type ChannelType = "slack" | "discord" | "matrix" | "msteams";
export type HimalayaBackend = "imap" | "jmap" | "gmail" | "msgraph" | "maildir" | "m2dir";
export type TeslaRegion = "north_america" | "europe" | "china";
export type TeslaDatasetMode = "state" | "history";
export type TeslaVehicleEndpoint =
	| "charge_state"
	| "climate_state"
	| "drive_state"
	| "location_data"
	| "vehicle_state"
	| "vehicle_config"
	| "gui_settings";
export type TeslaVehicleOnlineState = "online" | "asleep" | "offline" | "waking" | "unknown";

export interface ConnectorDescriptor {
	kind: ConnectorKind;
	displayName: string;
}

interface ConnectorAccountBase {
	id: string;
	name: string;
	managed: boolean;
	enabled: boolean;
	createdAt: string;
	updatedAt: string;
}

export interface CalDavConnectorAccount extends ConnectorAccountBase {
	kind: "caldav";
	serverUrl: string;
	username: string;
	timeoutSeconds: number;
	allowInsecureHttp: boolean;
	allowPrivateNetwork: boolean;
	hasPassword: boolean;
}

export interface ChannelHistoryConnectorAccount extends ConnectorAccountBase {
	kind: "channel_history";
	channelType: ChannelType;
	channelAccountId: string;
}

export interface GmailConnectorAccount extends ConnectorAccountBase {
	kind: "gmail";
	credentialSource: "google_workspace";
}

export interface HimalayaConnectorAccount extends ConnectorAccountBase {
	kind: "himalaya";
	himalayaAccountName: string;
	himalayaBackend: HimalayaBackend;
	credentialSource: "himalaya";
}

export interface TeslaConnectorAccount extends ConnectorAccountBase {
	kind: "tesla";
	teslaRegion: TeslaRegion;
	teslaClientId: string;
	hasPassword: boolean;
	credentialSource: "tesla_fleet_api";
}

export type ConnectorAccount =
	| CalDavConnectorAccount
	| ChannelHistoryConnectorAccount
	| GmailConnectorAccount
	| HimalayaConnectorAccount
	| TeslaConnectorAccount;

export interface ConnectorVehicle {
	vin: string;
	displayName?: string;
	state: TeslaVehicleOnlineState;
}

export interface ConnectorChannelSource {
	channelType: ChannelType;
	accountId: string;
	displayName: string;
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

export interface CalDavConnectorDatasetConfig {
	schemaVersion: number;
	selection: CalendarSelection;
	filters: ConnectorDatasetFilters;
}

export interface ChannelHistoryConnectorDatasetConfig {
	schemaVersion: number;
	channelId: string;
	threadId: string;
	limit: number;
}

export interface GmailConnectorDatasetConfig {
	schemaVersion: number;
	query: string;
	maxMessages: number;
	includeBody: boolean;
}

export interface HimalayaConnectorDatasetConfig {
	schemaVersion: number;
	mailboxIds: string[];
	query?: string | null;
	maxMessages: number;
	includeBodies: boolean;
}

export interface TeslaConnectorDatasetConfig {
	schemaVersion: number;
	mode: TeslaDatasetMode;
	vins: string[];
	endpoints: TeslaVehicleEndpoint[];
	maxSamples: number;
}

export type ConnectorDatasetConfig =
	| CalDavConnectorDatasetConfig
	| ChannelHistoryConnectorDatasetConfig
	| GmailConnectorDatasetConfig
	| HimalayaConnectorDatasetConfig
	| TeslaConnectorDatasetConfig;

export interface ConnectorProjections {
	jsonl: boolean;
	markdown: boolean;
}

interface ConnectorDatasetBase {
	id: string;
	accountId: string;
	name: string;
	instruction?: string;
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

export interface CalDavConnectorDataset extends ConnectorDatasetBase {
	kind: "caldav";
	config: CalDavConnectorDatasetConfig;
}

export interface ChannelHistoryConnectorDataset extends ConnectorDatasetBase {
	kind: "channel_history";
	config: ChannelHistoryConnectorDatasetConfig;
}

export interface GmailConnectorDataset extends ConnectorDatasetBase {
	kind: "gmail";
	config: GmailConnectorDatasetConfig;
}

export interface HimalayaConnectorDataset extends ConnectorDatasetBase {
	kind: "himalaya";
	config: HimalayaConnectorDatasetConfig;
}

export interface TeslaConnectorDataset extends ConnectorDatasetBase {
	kind: "tesla";
	config: TeslaConnectorDatasetConfig;
}

export type ConnectorDataset =
	| CalDavConnectorDataset
	| ChannelHistoryConnectorDataset
	| GmailConnectorDataset
	| HimalayaConnectorDataset
	| TeslaConnectorDataset;

export interface ConnectorDatasetDraft {
	name: string;
	config: CalDavConnectorDatasetConfig;
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

export interface ConnectorChannelReadyResponse {
	channelReady: boolean;
}

export interface ConnectorEmailReadyResponse {
	emailReady: boolean;
	emailAddress?: string;
	mailboxes?: Array<{ id: string; displayName?: string }>;
}

export interface ConnectorVehiclesResponse {
	vehicles: ConnectorVehicle[];
}

export type ConnectorAccountTestResponse =
	| ConnectorCalendarsResponse
	| ConnectorChannelReadyResponse
	| ConnectorEmailReadyResponse
	| ConnectorVehiclesResponse;

export interface ConnectorChannelSourcesResponse {
	sources: ConnectorChannelSource[];
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
