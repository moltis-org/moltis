/**
 * Live integration tests for remote sandbox backends.
 *
 * These tests create real sandboxes via the Vercel/Daytona APIs and require
 * credentials in the environment. They are skipped when credentials are absent.
 *
 * Required secrets (GitHub Actions):
 *   VERCEL_TOKEN      — Vercel access token (ver_...)
 *   VERCEL_TEAM_ID    — Vercel team ID (team_...)
 *   VERCEL_PROJECT_ID — Vercel project ID (prj_...)
 *   DAYTONA_API_KEY   — Daytona API key (optional, for Daytona tests)
 *
 * Run locally:
 *   VERCEL_TOKEN=ver_xxx VERCEL_TEAM_ID=team_xxx VERCEL_PROJECT_ID=prj_xxx npx playwright test e2e/specs/remote-sandbox-live.spec.js
 */

const { test, expect } = require("../base-test");

const VERCEL_TOKEN = process.env.VERCEL_TOKEN;
const VERCEL_TEAM_ID = process.env.VERCEL_TEAM_ID;
const VERCEL_PROJECT_ID = process.env.VERCEL_PROJECT_ID;
const DAYTONA_API_KEY = process.env.DAYTONA_API_KEY;

const VERCEL_API = "https://vercel.com/api";

test.describe("Vercel Sandbox live integration", () => {
	test.skip(
		!(VERCEL_TOKEN && VERCEL_PROJECT_ID),
		"VERCEL_TOKEN or VERCEL_PROJECT_ID not set — skipping live Vercel tests",
	);

	let sandboxId = null;

	test.afterEach(async () => {
		// Clean up: stop the sandbox if one was created.
		if (sandboxId) {
			const url = `${VERCEL_API}/v1/sandboxes/${sandboxId}/stop${VERCEL_TEAM_ID ? `?teamId=${VERCEL_TEAM_ID}` : ""}`;
			await fetch(url, {
				method: "POST",
				headers: {
					Authorization: `Bearer ${VERCEL_TOKEN}`,
					"Content-Type": "application/json",
				},
			}).catch(() => {});
			sandboxId = null;
		}
	});

	test("create sandbox, run command, and stop", async () => {
		// Create a sandbox.
		const createUrl = `${VERCEL_API}/v1/sandboxes${VERCEL_TEAM_ID ? `?teamId=${VERCEL_TEAM_ID}` : ""}`;
		const createResp = await fetch(createUrl, {
			method: "POST",
			headers: {
				Authorization: `Bearer ${VERCEL_TOKEN}`,
				"Content-Type": "application/json",
			},
			body: JSON.stringify({
				projectId: VERCEL_PROJECT_ID,
				runtime: "node24",
				timeout: 60000,
				resources: { vcpus: 1 },
			}),
		});

		expect(createResp.status).toBe(200);
		const createData = await createResp.json();
		sandboxId = createData.sandbox?.id;
		expect(sandboxId).toBeTruthy();

		// Wait for sandbox to reach running state.
		const deadline = Date.now() + 60000;
		let status = "";
		while (Date.now() < deadline) {
			const getUrl = `${VERCEL_API}/v1/sandboxes/${sandboxId}${VERCEL_TEAM_ID ? `?teamId=${VERCEL_TEAM_ID}` : ""}`;
			const getResp = await fetch(getUrl, {
				headers: { Authorization: `Bearer ${VERCEL_TOKEN}` },
			});
			const getData = await getResp.json();
			status = getData.sandbox?.status;
			if (status === "running") break;
			if (status === "failed" || status === "stopped") {
				throw new Error(`Sandbox entered terminal state: ${status}`);
			}
			await new Promise((r) => setTimeout(r, 1000));
		}
		expect(status).toBe("running");

		// Execute a command.
		const cmdUrl = `${VERCEL_API}/v1/sandboxes/${sandboxId}/cmd${VERCEL_TEAM_ID ? `?teamId=${VERCEL_TEAM_ID}` : ""}`;
		const cmdResp = await fetch(cmdUrl, {
			method: "POST",
			headers: {
				Authorization: `Bearer ${VERCEL_TOKEN}`,
				"Content-Type": "application/json",
			},
			body: JSON.stringify({
				command: "echo",
				args: ["-n", "hello-moltis"],
				cwd: "/vercel/sandbox",
				wait: true,
			}),
		});

		expect(cmdResp.status).toBe(200);
		const cmdText = await cmdResp.text();
		// NDJSON response — last line has the exit code.
		const lines = cmdText.trim().split("\n").filter(Boolean);
		const lastLine = JSON.parse(lines[lines.length - 1]);
		expect(lastLine.command.exitCode).toBe(0);

		// Fetch command logs to verify output.
		const cmdId = lastLine.command.id;
		const logsUrl = `${VERCEL_API}/v1/sandboxes/${sandboxId}/cmd/${cmdId}/logs${VERCEL_TEAM_ID ? `?teamId=${VERCEL_TEAM_ID}` : ""}`;
		const logsResp = await fetch(logsUrl, {
			headers: { Authorization: `Bearer ${VERCEL_TOKEN}` },
		});
		const logsText = await logsResp.text();
		expect(logsText).toContain("hello-moltis");

		// Stop the sandbox.
		const stopUrl = `${VERCEL_API}/v1/sandboxes/${sandboxId}/stop${VERCEL_TEAM_ID ? `?teamId=${VERCEL_TEAM_ID}` : ""}`;
		const stopResp = await fetch(stopUrl, {
			method: "POST",
			headers: {
				Authorization: `Bearer ${VERCEL_TOKEN}`,
				"Content-Type": "application/json",
			},
		});
		expect(stopResp.status).toBe(200);
		sandboxId = null; // already stopped
	});

	test("write and read file in sandbox", async () => {
		// Create sandbox.
		const createUrl = `${VERCEL_API}/v1/sandboxes${VERCEL_TEAM_ID ? `?teamId=${VERCEL_TEAM_ID}` : ""}`;
		const createResp = await fetch(createUrl, {
			method: "POST",
			headers: {
				Authorization: `Bearer ${VERCEL_TOKEN}`,
				"Content-Type": "application/json",
			},
			body: JSON.stringify({
				projectId: VERCEL_PROJECT_ID,
				runtime: "node24",
				timeout: 60000,
				resources: { vcpus: 1 },
			}),
		});
		const createData = await createResp.json();
		sandboxId = createData.sandbox?.id;
		expect(sandboxId).toBeTruthy();

		// Wait for running.
		const deadline = Date.now() + 60000;
		while (Date.now() < deadline) {
			const getResp = await fetch(
				`${VERCEL_API}/v1/sandboxes/${sandboxId}${VERCEL_TEAM_ID ? `?teamId=${VERCEL_TEAM_ID}` : ""}`,
				{ headers: { Authorization: `Bearer ${VERCEL_TOKEN}` } },
			);
			const getData = await getResp.json();
			if (getData.sandbox?.status === "running") break;
			await new Promise((r) => setTimeout(r, 1000));
		}

		// Write a file using the command API (simpler than gzipped tar for test).
		const writeCmd = `${VERCEL_API}/v1/sandboxes/${sandboxId}/cmd${VERCEL_TEAM_ID ? `?teamId=${VERCEL_TEAM_ID}` : ""}`;
		const writeResp = await fetch(writeCmd, {
			method: "POST",
			headers: {
				Authorization: `Bearer ${VERCEL_TOKEN}`,
				"Content-Type": "application/json",
			},
			body: JSON.stringify({
				command: "sh",
				args: ["-c", "echo 'moltis-test-content' > /tmp/test-file.txt"],
				cwd: "/vercel/sandbox",
				wait: true,
			}),
		});
		expect(writeResp.status).toBe(200);

		// Read the file back.
		const readUrl = `${VERCEL_API}/v1/sandboxes/${sandboxId}/fs/read${VERCEL_TEAM_ID ? `?teamId=${VERCEL_TEAM_ID}` : ""}`;
		const readResp = await fetch(readUrl, {
			method: "POST",
			headers: {
				Authorization: `Bearer ${VERCEL_TOKEN}`,
				"Content-Type": "application/json",
			},
			body: JSON.stringify({ path: "/tmp/test-file.txt" }),
		});
		expect(readResp.status).toBe(200);
		const content = await readResp.text();
		expect(content.trim()).toBe("moltis-test-content");
	});
});

test.describe("Daytona Sandbox live integration", () => {
	test.skip(!DAYTONA_API_KEY, "DAYTONA_API_KEY not set — skipping live Daytona tests");

	const DAYTONA_API = process.env.DAYTONA_API_URL || "https://app.daytona.io/api";
	let sandboxId = null;

	test.afterEach(async () => {
		if (sandboxId) {
			await fetch(`${DAYTONA_API}/sandbox/${sandboxId}`, {
				method: "DELETE",
				headers: {
					Authorization: `Bearer ${DAYTONA_API_KEY}`,
					"X-Daytona-Source": "moltis-e2e",
				},
			}).catch(() => {});
			sandboxId = null;
		}
	});

	test("create sandbox and execute command", { timeout: 120_000 }, async () => {
		// Create a sandbox (retry on transient 5xx errors from Daytona).
		let createResp;
		for (let attempt = 0; attempt < 3; attempt++) {
			createResp = await fetch(`${DAYTONA_API}/sandbox`, {
				method: "POST",
				headers: {
					Authorization: `Bearer ${DAYTONA_API_KEY}`,
					"Content-Type": "application/json",
					"X-Daytona-Source": "moltis-e2e",
				},
				body: JSON.stringify({}),
			});
			if (createResp.status < 500) break;
			await new Promise((r) => setTimeout(r, 3000));
		}

		expect(createResp.status).toBeLessThan(300);
		const createData = await createResp.json();
		sandboxId = createData.id;
		expect(sandboxId).toBeTruthy();

		// Wait for sandbox toolbox to become available (may take a few seconds after creation).
		const execDeadline = Date.now() + 60000;
		let execResp;
		while (Date.now() < execDeadline) {
			execResp = await fetch(`${DAYTONA_API}/toolbox/${sandboxId}/toolbox/process/execute`, {
				method: "POST",
				headers: {
					Authorization: `Bearer ${DAYTONA_API_KEY}`,
					"Content-Type": "application/json",
					"X-Daytona-Source": "moltis-e2e",
				},
				body: JSON.stringify({
					command: "echo hello-from-daytona",
					cwd: "/home/daytona",
					timeout: 30,
				}),
			});
			if (execResp.status === 200) break;
			// 404 or 503 means toolbox not ready yet.
			await new Promise((r) => setTimeout(r, 2000));
		}

		expect(execResp.status).toBe(200);
		const execData = await execResp.json();
		expect(execData.exitCode).toBe(0);
		expect(execData.result).toContain("hello-from-daytona");

		// Delete sandbox.
		const deleteResp = await fetch(`${DAYTONA_API}/sandbox/${sandboxId}`, {
			method: "DELETE",
			headers: {
				Authorization: `Bearer ${DAYTONA_API_KEY}`,
				"X-Daytona-Source": "moltis-e2e",
			},
		});
		expect(deleteResp.status).toBeLessThan(300);
		sandboxId = null;
	});
});
