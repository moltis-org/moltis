export type RepositorySourceKind = "https" | "ssh" | "local";

export interface HttpsRepositorySource {
	kind: "https";
	url: string;
	private: boolean;
	httpsCredentialId?: number;
}

export interface SshRepositorySource {
	kind: "ssh";
	remote: string;
	sshTargetId: number;
}

export interface LocalRepositorySource {
	kind: "local";
	path: string;
}

export type RepositorySource = HttpsRepositorySource | SshRepositorySource | LocalRepositorySource;

export interface RepositoryRequest {
	id?: string;
	alias: string;
	source: { kind: "https"; url: string; private: boolean } | { kind: "ssh"; remote: string } | LocalRepositorySource;
	ref: string;
	discovery: "explicit";
	httpsCredentialId?: number;
	sshTargetId?: number;
}

export interface RepositoryProjection {
	id: string;
	alias: string;
	source: RepositorySource;
	ref: string;
	discovery: "explicit";
}

export interface ExpectedCandidate {
	identity: string;
	digest: string;
}

export interface ManagedCandidate extends ExpectedCandidate {
	runtimeName: string;
	transport: string;
	command: string;
	args: string[];
	cwd?: string;
	envNames: string[];
	url?: string;
	headerNames: string[];
	approved: boolean;
	approvalBlocked: boolean;
	approvalBlockReason?: string;
	warnings: string[];
}

export interface RepositoryWarning {
	kind: string;
	sourceManifestPath: string;
	pluginName?: string;
	sourceName?: string;
}

export interface ReconciliationDiff {
	added: string[];
	updated: string[];
	unchanged: string[];
	removed: string[];
}

export interface RepositoryPreview {
	repository: RepositoryProjection;
	commit: string;
	candidates: ManagedCandidate[];
	warnings: RepositoryWarning[];
	diff?: ReconciliationDiff;
}

export interface ManagedServerStatus {
	name: string;
	state: string;
	enabled: boolean;
	transport: string;
	managed?: {
		repository_id: string;
		repository_alias: string;
		commit: string;
		approved: boolean;
		approval_blocked: boolean;
		approval_block_reason?: string;
		warning_kinds?: string[];
	};
}

export interface InstalledRepository {
	repository: RepositoryProjection;
	activeCommit?: string;
	previousCommit?: string;
	servers: ManagedServerStatus[];
}

export interface RepositoriesListResponse {
	repositories: InstalledRepository[];
}

export interface GitHttpsCredential {
	id: number;
	host: string;
	username: string;
	created_at: string;
	updated_at: string;
	encrypted: boolean;
}

export interface SshKeyMetadata {
	id: number;
	name: string;
	fingerprint: string;
	encrypted: boolean;
	target_count: number;
}

export interface SshTargetMetadata {
	id: number;
	label: string;
	target: string;
	port?: number;
	authMode: "system" | "managed";
	keyId?: number;
	keyName?: string;
	hasKnownHost: boolean;
}

export interface GitCredentialsResponse {
	credentials: GitHttpsCredential[];
	sshKeys: SshKeyMetadata[];
	sshTargets: SshTargetMetadata[];
}

export interface CredentialMutationResponse {
	credential: GitHttpsCredential;
	storageWarning?: string;
}

export function expectedCandidates(candidates: ManagedCandidate[]): ExpectedCandidate[] {
	return candidates.map(({ identity, digest }) => ({ identity, digest }));
}

export function sourceDescription(source: RepositorySource): string {
	switch (source.kind) {
		case "https":
			return source.url;
		case "ssh":
			return source.remote;
		case "local":
			return source.path;
	}
}
