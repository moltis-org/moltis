// Mock OAuth server for E2E testing.
// Implements /authorize (PKCE + state), /token (code exchange + refresh), and /calls (request log).
// Usage: node mock-oauth-server.js
// Prints JSON to stdout: { "port": <number> }

const http = require("node:http");
const crypto = require("node:crypto");

var calls = [];
// Map of state -> { challenge, redirectUri }
var pendingFlows = new Map();
// Tracks issued auth codes -> { state } so /token can verify
var issuedCodes = new Map();
// Whether /token should return errors (toggled via /config)
var tokenShouldFail = false;

function parseRequestUrl(req) {
	return new URL(req.url, "http://127.0.0.1");
}

function queryObject(searchParams) {
	var query = {};
	for (const [key, value] of searchParams.entries()) {
		query[key] = value;
	}
	return query;
}

function base64UrlEncode(buffer) {
	return buffer.toString("base64").replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function verifyPkce(verifier, challenge) {
	var hash = crypto.createHash("sha256").update(verifier).digest();
	var expected = base64UrlEncode(hash);
	return expected === challenge;
}

function parseBody(req) {
	return new Promise((resolve) => {
		var chunks = [];
		req.on("data", (c) => chunks.push(c));
		req.on("end", () => {
			var body = Buffer.concat(chunks).toString();
			resolve(new URLSearchParams(body));
		});
	});
}

function respond(res, status, body) {
	var json = JSON.stringify(body);
	res.writeHead(status, { "Content-Type": "application/json" });
	res.end(json);
}

function handleAuthorize(res, query) {
	if (!(query.client_id && query.redirect_uri && query.code_challenge && query.state)) {
		return respond(res, 400, {
			error: "invalid_request",
			error_description: "Missing required parameters: client_id, redirect_uri, code_challenge, state",
		});
	}
	if (query.code_challenge_method !== "S256") {
		return respond(res, 400, {
			error: "invalid_request",
			error_description: "Only S256 code_challenge_method is supported",
		});
	}

	pendingFlows.set(query.state, {
		challenge: query.code_challenge,
		redirectUri: query.redirect_uri,
	});

	var authCode = `mock-auth-code-${crypto.randomBytes(8).toString("hex")}`;
	issuedCodes.set(authCode, { state: query.state });

	var redirectUrl = new URL(query.redirect_uri);
	redirectUrl.searchParams.set("code", authCode);
	redirectUrl.searchParams.set("state", query.state);

	res.writeHead(302, { Location: redirectUrl.toString() });
	res.end();
}

function handleTokenExchange(res, body) {
	var authCode = body.get("code");
	var verifier = body.get("code_verifier");
	var clientId = body.get("client_id");

	if (!(authCode && verifier && clientId)) {
		return respond(res, 400, {
			error: "invalid_request",
			error_description: "Missing code, code_verifier, or client_id",
		});
	}

	var codeEntry = issuedCodes.get(authCode);
	if (!codeEntry) {
		return respond(res, 400, {
			error: "invalid_grant",
			error_description: "Unknown or expired authorization code",
		});
	}

	var flow = pendingFlows.get(codeEntry.state);
	if (!flow) {
		return respond(res, 400, {
			error: "invalid_grant",
			error_description: "No pending flow for this state",
		});
	}

	if (!verifyPkce(verifier, flow.challenge)) {
		return respond(res, 400, {
			error: "invalid_grant",
			error_description: "PKCE verification failed",
		});
	}

	issuedCodes.delete(authCode);
	pendingFlows.delete(codeEntry.state);

	return respond(res, 200, {
		access_token: `mock-access-token-${crypto.randomBytes(8).toString("hex")}`,
		refresh_token: `mock-refresh-token-${crypto.randomBytes(8).toString("hex")}`,
		token_type: "Bearer",
		expires_in: 3600,
	});
}

function handleTokenRefresh(res, body) {
	var refreshToken = body.get("refresh_token");
	if (!refreshToken) {
		return respond(res, 400, {
			error: "invalid_request",
			error_description: "Missing refresh_token",
		});
	}

	return respond(res, 200, {
		access_token: `mock-refreshed-token-${crypto.randomBytes(8).toString("hex")}`,
		refresh_token: `mock-refresh-token-${crypto.randomBytes(8).toString("hex")}`,
		token_type: "Bearer",
		expires_in: 3600,
	});
}

async function handleToken(res, req) {
	var body = await parseBody(req);
	var grantType = body.get("grant_type");

	if (tokenShouldFail) {
		return respond(res, 400, {
			error: "server_error",
			error_description: "Mock server configured to return errors",
		});
	}

	if (grantType === "authorization_code") {
		return handleTokenExchange(res, body);
	}
	if (grantType === "refresh_token") {
		return handleTokenRefresh(res, body);
	}

	return respond(res, 400, {
		error: "unsupported_grant_type",
		error_description: `Unsupported grant_type: ${grantType}`,
	});
}

async function handleConfig(req) {
	var configBody = await parseBody(req);
	if (configBody.has("token_should_fail")) {
		tokenShouldFail = configBody.get("token_should_fail") === "true";
	}
}

function handleReset() {
	calls = [];
	pendingFlows.clear();
	issuedCodes.clear();
	tokenShouldFail = false;
}

var server = http.createServer(async (req, res) => {
	var parsed = parseRequestUrl(req);
	var pathname = parsed.pathname;
	var query = queryObject(parsed.searchParams);

	calls.push({
		method: req.method,
		path: pathname,
		query,
		timestamp: Date.now(),
	});

	if (req.method === "GET" && pathname === "/authorize") {
		return handleAuthorize(res, query);
	}
	if (req.method === "POST" && pathname === "/token") {
		return handleToken(res, req);
	}
	if (req.method === "GET" && pathname === "/calls") {
		return respond(res, 200, calls);
	}
	if (req.method === "POST" && pathname === "/config") {
		await handleConfig(req);
		return respond(res, 200, { ok: true });
	}
	if (req.method === "POST" && pathname === "/reset") {
		handleReset();
		return respond(res, 200, { ok: true });
	}

	respond(res, 404, { error: "not_found" });
});

server.listen(0, "127.0.0.1", () => {
	var port = server.address().port;
	process.stdout.write(`${JSON.stringify({ port })}\n`);
});
