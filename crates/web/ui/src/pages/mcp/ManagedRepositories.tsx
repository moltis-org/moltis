import { signal } from "@preact/signals";
import type { VNode } from "preact";
import { useEffect } from "preact/hooks";
import { onEvent } from "../../events";
import { GitCredentials } from "./GitCredentials";
import { InstalledRepositories } from "./InstalledRepositories";
import { RepositoryInstaller } from "./RepositoryInstaller";
import { mcpRepositoryRpc } from "./rpc";
import type { GitCredentialsResponse, RepositoriesListResponse } from "./types";

const emptyCredentials: GitCredentialsResponse = { credentials: [], sshKeys: [], sshTargets: [] };
const repositoryData = signal<RepositoriesListResponse>({ repositories: [] });
const credentialData = signal<GitCredentialsResponse>(emptyCredentials);
const loading = signal(false);
const statusMessage = signal<{ message: string; error: boolean } | null>(null);

interface ManagedRepositoriesProps {
	onServersChanged: () => Promise<void>;
}

export function ManagedRepositories({ onServersChanged }: ManagedRepositoriesProps): VNode {
	async function refresh(): Promise<void> {
		loading.value = true;
		try {
			const [repositories, credentials] = await Promise.all([
				mcpRepositoryRpc<RepositoriesListResponse>("mcp.repositories.list", {}),
				mcpRepositoryRpc<GitCredentialsResponse>("mcp.git.credentials.list", {}),
			]);
			repositoryData.value = repositories;
			credentialData.value = credentials;
		} catch (error) {
			showMessage(error instanceof Error ? error.message : "Managed repository data failed to load", true);
		} finally {
			loading.value = false;
		}
	}

	async function refreshAfterOperation(): Promise<void> {
		await Promise.all([refresh(), onServersChanged()]);
	}

	function showMessage(message: string, error = false): void {
		statusMessage.value = { message, error };
	}

	useEffect(() => {
		refresh();
		const off = onEvent("mcp.status", () => {
			refresh();
		});
		return off;
	}, []);

	return (
		<div className="max-w-[960px] border-t border-[var(--border)] pt-5">
			<div className="mb-4 flex flex-wrap items-start justify-between gap-3">
				<div>
					<h2 className="text-lg font-medium text-[var(--text-strong)]">Managed repositories</h2>
					<p className="mt-1 max-w-[720px] text-xs text-[var(--muted)]">
						Discover MCP servers from Git or a server-side checkout, inspect sanitized configuration, then approve each
						exact commit and config digest.
					</p>
				</div>
				<button
					type="button"
					className="provider-btn provider-btn-secondary provider-btn-sm"
					onClick={refresh}
					disabled={loading.value}
				>
					{loading.value ? "Refreshing..." : "Refresh repositories"}
				</button>
			</div>
			{statusMessage.value && (
				<div
					role={statusMessage.value.error ? "alert" : "status"}
					className={`mb-4 rounded-lg border px-3 py-2 text-xs ${
						statusMessage.value.error
							? "border-[var(--error)] text-[var(--error)]"
							: "border-[var(--ok)] text-[var(--ok)]"
					}`}
				>
					{statusMessage.value.message}
				</div>
			)}
			<div className="grid gap-4">
				<InstalledRepositories
					repositories={repositoryData.value.repositories}
					onChanged={refreshAfterOperation}
					onMessage={showMessage}
				/>
				<RepositoryInstaller
					credentials={credentialData.value}
					onChanged={refreshAfterOperation}
					onMessage={showMessage}
				/>
				<GitCredentials data={credentialData.value} onChanged={refresh} onMessage={showMessage} />
			</div>
		</div>
	);
}
