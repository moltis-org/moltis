const { expect, test } = require("../base-test");
const { navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

const OLD_COMMIT = "1111111111111111111111111111111111111111";
const PREVIOUS_COMMIT = "0000000000000000000000000000000000000000";
const NEW_COMMIT = "2222222222222222222222222222222222222222";
const INSTALL_COMMIT = "3333333333333333333333333333333333333333";

async function mockManagedRepositoriesRpc(page) {
	await page.route("**/api/mcp", async (route) => {
		await route.fulfill({ status: 200, contentType: "application/json", body: "[]" });
	});
	await page.addInitScript(
		({ oldCommit, previousCommit, newCommit, installCommit }) => {
			window.__mcpRepositoryE2ERequests = [];
			window.__mcpRepositoryE2EUpdateAvailable = false;
			const originalSend = WebSocket.prototype.send;
			const makeCandidate = (name, commit = installCommit) => ({
				runtimeName: name,
				identity: `${name}-identity`,
				digest: `${commit.slice(0, 8)}-${name}-digest`,
				transport: name === "yolo-8" ? "streamable-http" : "stdio",
				command: name === "yolo-8" ? "" : "npx",
				args: name === "yolo-1" ? ["-y", "runner", "--token", "[REDACTED]"] : ["-y", name],
				cwd: `servers/${name}`,
				envNames: name === "yolo-2" ? ["API_TOKEN"] : [],
				url: name === "yolo-8" ? "https://mcp.example.test/mcp?token=[REDACTED]" : undefined,
				headerNames: name === "yolo-8" ? ["Authorization"] : [],
				approved: false,
				approvalBlocked: name === "yolo-8",
				approvalBlockReason: name === "yolo-8" ? "repository manifest contains an unbound environment placeholder" : undefined,
				warnings: name === "yolo-3" ? ["shell-command"] : [],
			});
			const server = (name, repositoryId, alias, commit, approved = false, enabled = false) => ({
				name,
				state: enabled ? "running" : "stopped",
				enabled,
				transport: "stdio",
				managed: {
					repository_id: repositoryId,
					repository_alias: alias,
					commit,
					approved,
					approval_blocked: false,
					warning_kinds: name === "legacy-tool" ? ["legacy-manifest"] : [],
				},
			});
			const entry = ({ id, alias, commit, previous, servers, source }) => ({
				repository: {
					id,
					alias,
					source: source || { kind: "https", url: `https://github.com/example/${id}.git`, private: false },
					ref: "main",
					discovery: "explicit",
				},
				activeCommit: commit,
				previousCommit: previous,
				servers,
			});
			let repositories = [
				entry({
					id: "old-tools-v1",
					alias: "Old tools",
					commit: oldCommit,
					previous: previousCommit,
					servers: [server("legacy-tool", "old-tools-v1", "Old tools", oldCommit)],
				}),
			];
			let credentials = [
				{
					id: 7,
					host: "git.example.test",
					username: "deploy-bot",
					created_at: "2026-07-29T00:00:00Z",
					updated_at: "2026-07-29T00:00:00Z",
					encrypted: false,
				},
			];
			const sshTargets = [
				{
					id: 9,
					label: "Git production",
					target: "git@git.example.test",
					port: 22,
					authMode: "managed",
					keyId: 3,
					keyName: "deployment-key",
					hasKnownHost: true,
				},
			];

			function respond(socket, id, payload) {
				queueMicrotask(() => {
					const event = new MessageEvent("message", {
						data: JSON.stringify({ type: "res", id, ok: true, payload }),
					});
					if (typeof socket.onmessage === "function") socket.onmessage(event);
				});
			}

			WebSocket.prototype.send = function (raw) {
				let request;
				try {
					request = JSON.parse(raw);
				} catch {
					return originalSend.call(this, raw);
				}
				if (
					!(request?.method?.startsWith("mcp.repositories") || request?.method?.startsWith("mcp.git.credentials")) &&
					request?.method !== "mcp.managed.approve"
				) {
					return originalSend.call(this, raw);
				}
				window.__mcpRepositoryE2ERequests.push({ method: request.method, params: request.params || {} });
				switch (request.method) {
					case "mcp.repositories.list":
						respond(this, request.id, { repositories });
						return;
					case "mcp.git.credentials.list":
						respond(this, request.id, { credentials, sshKeys: [], sshTargets });
						return;
					case "mcp.repositories.preview": {
						const candidates = Array.from({ length: 8 }, (_, index) => makeCandidate(`yolo-${index + 1}`));
						respond(this, request.id, {
							repository: {
								id: request.params.id || "yolo-repo-v1",
								alias: request.params.alias,
								source: { ...request.params.source, httpsCredentialId: request.params.httpsCredentialId },
								ref: request.params.ref,
								discovery: "explicit",
							},
							commit: installCommit,
							candidates,
							warnings: [{ kind: "yolo-manifest", sourceManifestPath: ".mcp.json" }],
						});
						return;
					}
					case "mcp.repositories.install": {
						const selected = request.params.selection.candidates;
						repositories = [
							...repositories,
							entry({
								id: request.params.id || "yolo-repo-v1",
								alias: request.params.alias,
								commit: installCommit,
								servers: selected.map((item) =>
									server(item.identity.replace(/-identity$/, ""), "yolo-repo-v1", request.params.alias, installCommit),
								),
								source: { ...request.params.source, httpsCredentialId: request.params.httpsCredentialId },
							}),
						];
						respond(this, request.id, { commit: installCommit });
						return;
					}
					case "mcp.repositories.update.preview": {
						const isUpdate = window.__mcpRepositoryE2EUpdateAvailable;
						const commit = isUpdate ? newCommit : oldCommit;
						const candidates = isUpdate
							? [makeCandidate("legacy-tool", commit), makeCandidate("added-tool", commit)]
							: [makeCandidate("legacy-tool", commit)];
						respond(this, request.id, {
							repository: repositories.find((item) => item.repository.id === request.params.id)?.repository,
							commit,
							candidates,
							warnings: [],
							diff: isUpdate
								? { added: ["added-tool"], updated: ["legacy-tool"], removed: ["removed-tool"], unchanged: [] }
								: { added: [], updated: [], removed: [], unchanged: ["legacy-tool"] },
						});
						return;
					}
					case "mcp.managed.approve": {
						const current = repositories.find((item) => item.repository.id === request.params.id);
						if (current) {
							const selected = new Set(request.params.selection.candidates.map((item) => item.identity));
							current.servers = current.servers.map((item) => ({
								...item,
								enabled: selected.has(`${item.name}-identity`) ? request.params.enable : item.enabled,
								managed: { ...item.managed, approved: selected.has(`${item.name}-identity`) || item.managed.approved },
							}));
						}
						respond(this, request.id, {
							approved: request.params.selection.candidates,
							enabled: request.params.enable,
						});
						return;
					}
					case "mcp.repositories.update.apply": {
						const current = repositories.find((item) => item.repository.id === request.params.id);
						if (current) {
							current.previousCommit = current.activeCommit;
							current.activeCommit = newCommit;
							current.servers = [
								server("legacy-tool", current.repository.id, current.repository.alias, newCommit),
								server("added-tool", current.repository.id, current.repository.alias, newCommit),
							];
						}
						window.__mcpRepositoryE2EUpdateAvailable = false;
						respond(this, request.id, { commit: newCommit });
						return;
					}
					case "mcp.repositories.rollback": {
						const current = repositories.find((item) => item.repository.id === request.params.id);
						if (current) current.activeCommit = request.params.expectedCommit;
						respond(this, request.id, { commit: request.params.expectedCommit });
						return;
					}
					case "mcp.repositories.remove":
						repositories = repositories.filter((item) => item.repository.id !== request.params.id);
						respond(this, request.id, { removed: true });
						return;
					case "mcp.git.credentials.create": {
						const credential = {
							id: 8,
							host: request.params.host,
							username: request.params.username,
							created_at: "2026-07-29T00:00:00Z",
							updated_at: "2026-07-29T00:00:00Z",
							encrypted: true,
						};
						credentials = [...credentials, credential];
						respond(this, request.id, { credential });
						return;
					}
					default:
						respond(this, request.id, { removed: true });
				}
			};
		},
		{ oldCommit: OLD_COMMIT, previousCommit: PREVIOUS_COMMIT, newCommit: NEW_COMMIT, installCommit: INSTALL_COMMIT },
	);
}

test.describe("Managed MCP repositories", () => {
	test("previews, selectively imports, approves, updates, rolls back, and removes", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await mockManagedRepositoriesRpc(page);
		await navigateAndWait(page, "/settings/mcp");
		await waitForWsConnected(page);
		await page.getByRole("button", { name: "Refresh repositories", exact: true }).click();

		await expect(page.getByRole("heading", { name: "Managed repositories", exact: true })).toBeVisible();
		await expect(page.getByRole("button", { name: "Install selected", exact: false })).toHaveCount(0);

		const addSection = page
			.getByRole("heading", { name: "Add managed repository", exact: true })
			.locator("..")
			.locator("..");
		await addSection.getByLabel("Repository source", { exact: true }).fill("https://github.com/example/yolo-tools.git");
		await addSection.getByLabel("Alias", { exact: true }).fill("Yolo tools");
		await addSection.getByLabel("Git ref", { exact: true }).fill("main");
		await addSection.getByRole("button", { name: "Preview repository", exact: true }).click();

		await expect(page.getByRole("heading", { name: "Repository preview", exact: true })).toBeVisible();
		await expect(page.getByText(`Commit ${INSTALL_COMMIT}`, { exact: true })).toBeVisible();
		await expect(page.getByText("yolo-manifest: .mcp.json", { exact: true })).toBeVisible();
		for (let index = 1; index <= 8; index++) {
			const checkbox = page.getByRole("checkbox", { name: `Select yolo-${index}`, exact: true });
			if (index === 8) await expect(checkbox).toBeDisabled();
			else await expect(checkbox).toBeChecked();
		}
		await expect(page.getByText("approval blocked", { exact: true })).toBeVisible();
		await expect(
			page.getByText("repository manifest contains an unbound environment placeholder", { exact: true }),
		).toBeVisible();
		await expect(page.getByText("npx -y runner --token [REDACTED]", { exact: true })).toBeVisible();
		await expect(page.getByText("Authorization", { exact: false })).toBeVisible();

		await addSection.getByLabel("Git ref", { exact: true }).fill("release");
		await expect(page.getByRole("button", { name: "Install all", exact: false })).toHaveCount(0);
		await addSection.getByRole("button", { name: "Preview repository", exact: true }).click();
		for (let index = 3; index <= 7; index++) {
			await page.getByRole("checkbox", { name: `Select yolo-${index}`, exact: true }).uncheck();
		}
		await page.getByRole("checkbox", { name: "Select yolo-2", exact: true }).uncheck();
		await page.getByRole("button", { name: "Install selected (1)", exact: true }).click();

		const imported = page
			.locator("article")
			.filter({ has: page.getByRole("heading", { name: "Yolo tools", exact: true }) });
		await expect(imported.getByText("Approved 0/1", { exact: true })).toBeVisible();
		await expect(imported.getByText("Enabled 0/1", { exact: true })).toBeVisible();
		const installRequest = await page.evaluate(() =>
			window.__mcpRepositoryE2ERequests.find((request) => request.method === "mcp.repositories.install"),
		);
		expect(installRequest.params.expectedCommit).toBe(INSTALL_COMMIT);
		expect(installRequest.params.selection.mode).toBe("selected");
		expect(installRequest.params.selection.candidates).toHaveLength(1);

		const oldRepository = page
			.locator("article")
			.filter({ has: page.getByRole("heading", { name: "Old tools", exact: true }) });
		await oldRepository.getByRole("button", { name: "Approve selected", exact: true }).click();
		await expect(oldRepository.getByText("Approved 1/1", { exact: true })).toBeVisible();
		await oldRepository.getByRole("button", { name: "Approve and enable all", exact: true }).click();
		await expect(oldRepository.getByText("Enabled 1/1", { exact: true })).toBeVisible();

		await page.evaluate(() => {
			window.__mcpRepositoryE2EUpdateAvailable = true;
		});
		await oldRepository.getByRole("button", { name: "Preview update", exact: true }).click();
		await expect(oldRepository.getByText("added-tool", { exact: true })).toBeVisible();
		await expect(oldRepository.getByText("legacy-tool", { exact: true }).first()).toBeVisible();
		await expect(oldRepository.getByText("removed-tool", { exact: false })).toBeVisible();
		await oldRepository.getByRole("button", { name: "Apply update", exact: true }).click();
		await expect(oldRepository.getByText("Approved 0/2", { exact: true })).toBeVisible();

		await oldRepository.getByRole("button", { name: "Rollback", exact: true }).click();
		await page.locator(".modal-box").getByRole("button", { name: "Rollback", exact: true }).click();
		await expect(oldRepository.getByText(`Active: ${OLD_COMMIT}`, { exact: true })).toBeVisible();

		await oldRepository.getByRole("button", { name: "Remove repository", exact: true }).click();
		await page.locator(".modal-box").getByRole("button", { name: "Remove", exact: true }).click();
		await expect(page.getByRole("heading", { name: "Old tools", exact: true })).toHaveCount(0);
		expect(pageErrors).toEqual([]);
	});

	test("keeps HTTPS tokens and SSH key material out of rendered metadata", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await mockManagedRepositoriesRpc(page);
		await navigateAndWait(page, "/settings/mcp");
		await waitForWsConnected(page);
		await page.getByRole("button", { name: "Refresh repositories", exact: true }).click();

		await expect(
			page.getByText("Plaintext storage: vault encryption unavailable or sealed", { exact: true }),
		).toBeVisible();
		await expect(page.getByText("Host pin available", { exact: true })).toBeVisible();
		await expect(page.getByText("deployment-key", { exact: false })).toBeVisible();
		await expect(page.getByText("PRIVATE KEY", { exact: false })).toHaveCount(0);
		await expect(page.getByText("known_hosts", { exact: false })).toHaveCount(0);

		const secret = "credential-token-must-never-render";
		await page.getByLabel("Git host", { exact: true }).fill("new.example.test");
		await page.getByLabel("Username", { exact: true }).fill("new-bot");
		const tokenInput = page.getByLabel("Access token", { exact: true });
		await expect(tokenInput).toHaveAttribute("type", "password");
		await tokenInput.fill(secret);
		await page.getByRole("button", { name: "Create credential", exact: true }).click();
		await expect(page.getByText("new-bot@new.example.test", { exact: true })).toBeVisible();
		await expect(tokenInput).toHaveValue("");
		await expect(page.getByText(secret, { exact: false })).toHaveCount(0);
		const createRequest = await page.evaluate(() =>
			window.__mcpRepositoryE2ERequests.find((request) => request.method === "mcp.git.credentials.create"),
		);
		expect(createRequest.params.token).toBe(secret);
		expect(pageErrors).toEqual([]);
	});
});
