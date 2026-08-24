export type FileEntryKind = "file" | "directory" | "other";

export interface FileEntry {
	name: string;
	path: string;
	kind: FileEntryKind;
	sizeBytes: number | null;
	modifiedAt: string | null;
}

export interface FileEntriesResponse {
	path: string;
	entries: FileEntry[];
}

export class FilesApiError extends Error {
	readonly status: number;

	constructor(status: number, message: string) {
		super(message);
		this.name = "FilesApiError";
		this.status = status;
	}
}

function pathHeader(path: string): string {
	const bytes = new TextEncoder().encode(path);
	let binary = "";
	for (const byte of bytes) binary += String.fromCharCode(byte);
	return `base64url:${btoa(binary).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "")}`;
}

function apiErrorMessage(value: unknown): string | null {
	if (typeof value !== "object" || value === null) return null;
	if ("message" in value && typeof value.message === "string") return value.message;
	if ("error" in value && typeof value.error === "string") return value.error;
	return null;
}

async function responseError(response: Response, fallback: string): Promise<FilesApiError> {
	const text = await response.text().catch(() => "");
	let message = text.trim();
	if (message) {
		try {
			const parsed: unknown = JSON.parse(message);
			message = apiErrorMessage(parsed) ?? message;
		} catch {
			// Plain-text API errors are already suitable for display.
		}
	}
	return new FilesApiError(response.status, message || fallback);
}

async function requireOk(response: Response, fallback: string): Promise<Response> {
	if (!response.ok) throw await responseError(response, fallback);
	return response;
}

export function isConflict(error: unknown): error is FilesApiError {
	return error instanceof FilesApiError && error.status === 409;
}

export async function listEntries(path: string, signal?: AbortSignal): Promise<FileEntriesResponse> {
	const response = await fetch("/api/files/entries", {
		headers: { "X-Moltis-File-Path": pathHeader(path) },
		signal,
	});
	await requireOk(response, "Could not load files");
	return (await response.json()) as FileEntriesResponse;
}

export async function uploadFile(path: string, file: File, overwrite = false, signal?: AbortSignal): Promise<void> {
	const headers: Record<string, string> = {
		"Content-Type": file.type || "application/octet-stream",
		"X-Moltis-File-Path": pathHeader(path),
	};
	if (overwrite) headers["X-Moltis-Overwrite"] = "true";
	const response = await fetch("/api/files/content", {
		method: "PUT",
		headers,
		body: file,
		signal,
	});
	await requireOk(response, `Could not upload ${file.name}`);
}

export async function downloadFile(path: string): Promise<Response> {
	const response = await fetch("/api/files/content", {
		headers: { "X-Moltis-File-Path": pathHeader(path) },
	});
	return requireOk(response, "Could not download file");
}

export async function bufferDownload(response: Response, maxBytes: number): Promise<Blob> {
	const declaredSize = Number(response.headers.get("Content-Length"));
	if (!Number.isSafeInteger(declaredSize) || declaredSize < 0 || declaredSize > maxBytes) {
		throw new Error("This browser cannot safely buffer this file. Use a Chromium-based browser to download it.");
	}
	if (!response.body) throw new Error("The download response did not contain a file stream.");

	const reader = response.body.getReader();
	const chunks: BlobPart[] = [];
	let received = 0;
	for (;;) {
		const { done, value } = await reader.read();
		if (done) break;
		received += value.byteLength;
		if (received > maxBytes) {
			await reader.cancel();
			throw new Error("This browser cannot safely buffer this file. Use a Chromium-based browser to download it.");
		}
		chunks.push(new Uint8Array(value).buffer);
	}
	return new Blob(chunks, { type: response.headers.get("Content-Type") ?? "application/octet-stream" });
}

export async function createDirectory(path: string): Promise<void> {
	const response = await fetch("/api/files/directories", {
		method: "POST",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify({ path }),
	});
	await requireOk(response, "Could not create folder");
}

export async function moveEntry(source: string, destination: string, overwrite = false): Promise<void> {
	const response = await fetch("/api/files/move", {
		method: "POST",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify({ source, destination, overwrite }),
	});
	await requireOk(response, "Could not move entry");
}

export async function deleteEntry(path: string, recursive: boolean): Promise<void> {
	const response = await fetch("/api/files/entries", {
		method: "DELETE",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify({ path, recursive }),
	});
	await requireOk(response, "Could not delete entry");
}
