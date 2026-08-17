import type { VNode } from "preact";
import { CheckboxField, SelectField, TextAreaField, TextField } from "../../components/forms";
import { useTranslation } from "../../i18n";
import type {
	ConnectorDataset,
	TeslaConnectorDatasetConfig,
	TeslaDatasetMode,
	TeslaVehicleEndpoint,
} from "../../types/connector";

const MAX_VINS = 50;
const VIN_LENGTH = 17;
const MIN_SAMPLES = 1;
const MAX_SAMPLES = 20_000;

export const TESLA_ENDPOINTS: TeslaVehicleEndpoint[] = [
	"charge_state",
	"climate_state",
	"drive_state",
	"vehicle_state",
	"vehicle_config",
	"gui_settings",
	"location_data",
];

export interface TeslaDatasetValues {
	mode: TeslaDatasetMode;
	vins: string;
	endpoints: TeslaVehicleEndpoint[];
	maxSamples: string;
}

export function defaultTeslaValues(dataset: ConnectorDataset | null): TeslaDatasetValues {
	if (dataset?.kind === "tesla") {
		return {
			mode: dataset.config.mode,
			vins: dataset.config.vins.join("\n"),
			endpoints: dataset.config.endpoints,
			maxSamples: String(dataset.config.maxSamples),
		};
	}
	return {
		mode: "state",
		vins: "",
		endpoints: ["charge_state", "climate_state", "drive_state", "vehicle_state"],
		maxSamples: "2000",
	};
}

export interface TeslaDatasetPayload {
	instruction: string;
	config: TeslaConnectorDatasetConfig;
}

/**
 * Validates the form values and builds the request payload. Returns a
 * translation key instead of throwing so the caller can surface the message.
 */
export function buildTeslaDatasetPayload(
	values: TeslaDatasetValues,
): { payload: TeslaDatasetPayload } | { errorKey: string } {
	const vins = values.vins
		.split(/\r?\n/)
		.map((value) => value.trim().toUpperCase())
		.filter(Boolean);
	if (vins.length > MAX_VINS || vins.some((vin) => vin.length !== VIN_LENGTH || !/^[0-9A-Z]+$/.test(vin))) {
		return { errorKey: "datasets.teslaVinsInvalid" };
	}
	if (new Set(vins).size !== vins.length) {
		return { errorKey: "datasets.teslaVinsInvalid" };
	}
	if (values.endpoints.length === 0) {
		return { errorKey: "datasets.teslaEndpointsRequired" };
	}
	const maxSamples = Number(values.maxSamples);
	if (!(Number.isSafeInteger(maxSamples) && maxSamples >= MIN_SAMPLES && maxSamples <= MAX_SAMPLES)) {
		return { errorKey: "datasets.teslaSamplesInvalid" };
	}

	const scope = vins.length > 0 ? vins.join(", ") : "every vehicle on the account";
	return {
		payload: {
			instruction:
				values.mode === "history"
					? `Keep up to ${maxSamples} readings per vehicle for ${scope}.`
					: `Keep the current state of ${scope}.`,
			config: {
				schemaVersion: 1,
				mode: values.mode,
				vins,
				endpoints: values.endpoints,
				maxSamples,
			},
		},
	};
}

export interface TeslaDatasetFieldsProps {
	values: TeslaDatasetValues;
	onChange: (values: TeslaDatasetValues) => void;
}

export function TeslaDatasetFields({ values, onChange }: TeslaDatasetFieldsProps): VNode {
	const { t } = useTranslation("connectors");

	function toggleEndpoint(endpoint: TeslaVehicleEndpoint, checked: boolean): void {
		const endpoints = checked
			? [...values.endpoints, endpoint]
			: values.endpoints.filter((current) => current !== endpoint);
		onChange({ ...values, endpoints });
	}

	return (
		<>
			<SelectField
				id="connector-tesla-mode"
				label={t("datasets.teslaMode")}
				value={values.mode}
				onChange={(value) => onChange({ ...values, mode: value as TeslaDatasetMode })}
				options={[
					{ value: "state", label: t("datasets.teslaModeState") },
					{ value: "history", label: t("datasets.teslaModeHistory") },
				]}
			/>
			<div className="mb-3 text-xs text-[var(--muted)]">
				{t(values.mode === "history" ? "datasets.teslaModeHistoryHelp" : "datasets.teslaModeStateHelp")}
			</div>
			<TextAreaField
				id="connector-tesla-vins"
				label={t("datasets.teslaVins")}
				value={values.vins}
				onInput={(value) => onChange({ ...values, vins: value })}
				rows={3}
				monospace
				placeholder={t("datasets.teslaVinsPlaceholder")}
				help={t("datasets.teslaVinsHelp")}
			/>
			<div className="mb-1 text-xs font-medium text-[var(--text-strong)]">{t("datasets.teslaEndpoints")}</div>
			{TESLA_ENDPOINTS.map((endpoint) => (
				<CheckboxField
					key={endpoint}
					label={t(`datasets.teslaEndpointNames.${endpoint}`)}
					checked={values.endpoints.includes(endpoint)}
					onChange={(checked) => toggleEndpoint(endpoint, checked)}
				/>
			))}
			<div className="mb-3 text-xs text-[var(--muted)]">{t("datasets.teslaLocationHelp")}</div>
			{values.mode === "history" ? (
				<TextField
					id="connector-tesla-max-samples"
					label={t("datasets.teslaMaxSamples")}
					value={values.maxSamples}
					onInput={(value) => onChange({ ...values, maxSamples: value })}
					help={t("datasets.teslaMaxSamplesHelp")}
				/>
			) : null}
			<div className="mb-3 rounded border border-[var(--border)] bg-[var(--bg)] p-3 text-xs text-[var(--muted)]">
				{t("datasets.teslaSleepNote")}
			</div>
		</>
	);
}
