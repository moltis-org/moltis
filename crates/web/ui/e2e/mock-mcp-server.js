// Mock MCP Streamable HTTP server for E2E testing.
// Implements the MCP protocol over HTTP POST (JSON-RPC 2.0).
// Validates Bearer token auth when configured.
// Returns 405 for GET by default (many real Streamable HTTP servers do this).
//
// Usage: node mock-mcp-server.js [--bearer-token TOKEN] [--get-status CODE]
// Prints JSON to stdout: { "port": <number> }

const http = require("node:http");

var bearerToken = null;
var getStatusCode = 405;
var tools = [
	{
		name: "mock_echo",
		description: "Echoes the input back",
		inputSchema: { type: "object", properties: { message: { type: "string" } } },
	},
];

// Parse CLI args
for (var i = 2; i < process.argv.length; i++) {
	if (process.argv[i] === "--bearer-token" && process.argv[i + 1]) {
		bearerToken = process.argv[++i];
	}
	if (process.argv[i] === "--get-status" && process.argv[i + 1]) {
		getStatusCode = parseInt(process.argv[++i], 10);
	}
}

function respond(res, status, body, extraHeaders) {
	var json = JSON.stringify(body);
	var headers = { "Content-Type": "application/json", ...extraHeaders };
	res.writeHead(status, headers);
	res.end(json);
}

function parseBody(req) {
	return new Promise((resolve) => {
		var chunks = [];
		req.on("data", (c) => chunks.push(c));
		req.on("end", () => {
			var body = Buffer.concat(chunks).toString();
			try {
				resolve(JSON.parse(body));
			} catch {
				resolve(null);
			}
		});
	});
}

function checkAuth(req, res) {
	if (!bearerToken) return true;
	var auth = req.headers["authorization"] || "";
	if (auth === `Bearer ${bearerToken}`) return true;
	respond(res, 401, { error: "unauthorized" }, { "WWW-Authenticate": 'Bearer realm="mock-mcp"' });
	return false;
}

function handleJsonRpc(rpcBody) {
	var method = rpcBody.method;
	var id = rpcBody.id;

	if (method === "initialize") {
		return {
			jsonrpc: "2.0",
			id,
			result: {
				protocolVersion: "2025-03-26",
				capabilities: { tools: {} },
				serverInfo: { name: "mock-mcp-server", version: "1.0.0" },
			},
		};
	}

	if (method === "notifications/initialized") {
		// Notification — no response needed, but we send 202 from caller
		return null;
	}

	if (method === "tools/list") {
		return {
			jsonrpc: "2.0",
			id,
			result: { tools },
		};
	}

	if (method === "tools/call") {
		var args = rpcBody.params?.arguments || {};
		return {
			jsonrpc: "2.0",
			id,
			result: {
				content: [{ type: "text", text: `echo: ${args.message || "(empty)"}` }],
			},
		};
	}

	return {
		jsonrpc: "2.0",
		id,
		error: { code: -32601, message: `Method not found: ${method}` },
	};
}

var server = http.createServer(async (req, res) => {
	// GET: health check probe from Moltis. Many real Streamable HTTP servers
	// return 405 here because GET is optional in the MCP spec.
	if (req.method === "GET") {
		if (req.url === "/health") {
			return respond(res, 200, { ok: true });
		}
		res.writeHead(getStatusCode, { Allow: "POST, DELETE" });
		res.end();
		return;
	}

	// DELETE: session termination
	if (req.method === "DELETE") {
		res.writeHead(204);
		res.end();
		return;
	}

	// POST: JSON-RPC requests
	if (req.method === "POST") {
		if (!checkAuth(req, res)) return;

		var body = await parseBody(req);
		if (!body || !body.jsonrpc) {
			return respond(res, 400, { error: "invalid JSON-RPC request" });
		}

		var rpcResponse = handleJsonRpc(body);
		if (rpcResponse === null) {
			// Notification — no response body
			res.writeHead(202);
			res.end();
			return;
		}

		return respond(res, 200, rpcResponse);
	}

	res.writeHead(405, { Allow: "GET, POST, DELETE" });
	res.end();
});

server.listen(0, "127.0.0.1", () => {
	var port = server.address().port;
	process.stdout.write(`${JSON.stringify({ port })}\n`);
});
