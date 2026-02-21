# GraphQL API

Moltis exposes a GraphQL API that mirrors gateway RPC methods with typed query
and mutation responses where the data shape is known.

## Availability

GraphQL is compile-time feature gated:

- Gateway feature: `graphql`
- CLI feature: `graphql` (enabled in default feature set)

If Moltis is built without this feature, `/graphql` is not registered.

When built with the feature, GraphQL is runtime-toggleable:

- Config: `[graphql] enabled = true|false`
- UI: `Settings > GraphQL` toggle

Changes apply immediately, without restart.

## Endpoints

| Method | Path | Purpose |
|--------|------|---------|
| `GET` | `/graphql` | GraphiQL playground and WebSocket subscriptions |
| `POST` | `/graphql` | Queries and mutations |

WebSocket subprotocols accepted:

- `graphql-transport-ws`
- `graphql-ws`

## Authentication and Security

GraphQL is protected by the same auth decisions used elsewhere in the gateway.
It is not on the public path allowlist.

- With `web-ui` builds, GraphQL is behind the global `auth_gate` middleware.
- Without `web-ui`, GraphQL is explicitly guarded by `graphql_auth_gate`.

When auth is required and the request is unauthenticated, GraphQL returns `401`
(`{"error":"not authenticated"}` or `{"error":"setup required"}`).

When GraphQL is runtime-disabled, `/graphql` returns `503`
(`{"error":"graphql server is disabled"}`).

Supported auth methods:

- Valid session cookie (`moltis_session`)
- `Authorization: Bearer <api_key>`

```admonish warning
Do not expose Moltis to untrusted networks with authentication disabled.
```

## Schema Layout

The schema is organized by namespaces that map to gateway method groups.

Top-level query fields include:

- `health`
- `status`
- `system`, `node`, `chat`, `sessions`, `channels`
- `config`, `cron`, `heartbeat`, `logs`
- `tts`, `stt`, `voice`
- `skills`, `models`, `providers`, `mcp`
- `usage`, `execApprovals`, `projects`, `memory`, `hooks`, `agents`
- `voicewake`, `device`

Top-level mutation fields follow the same namespace pattern (for example:
`chat.send`, `config.set`, `cron.add`, `providers.oauthStart`, `mcp.reauth`).

Subscriptions include:

- `chatEvent`
- `sessionChanged`
- `cronNotification`
- `channelEvent`
- `nodeEvent`
- `tick`
- `logEntry`
- `mcpStatusChanged`
- `approvalEvent`
- `configChanged`
- `presenceChanged`
- `metricsUpdate`
- `updateAvailable`
- `voiceConfigChanged`
- `skillsInstallProgress`
- `allEvents`

## Typed Data and `Json` Scalar

Most GraphQL return types are concrete structs. The custom `Json` scalar is
still used where runtime shape is intentionally dynamic (for example: arbitrary
config values, context payloads, or pass-through node payloads).

## Examples

### Query (health)

```bash
curl -sS http://localhost:13131/graphql \
  -H 'content-type: application/json' \
  -H 'authorization: Bearer mk_your_api_key' \
  -d '{"query":"{ health { ok connections } }"}'
```

### Query with namespace fields

```graphql
query {
  status {
    hostname
    version
    connections
  }
  cron {
    list {
      id
      name
      enabled
    }
  }
}
```

### Mutation

```graphql
mutation {
  chat {
    send(message: "Hello from GraphQL") {
      ok
      sessionKey
    }
  }
}
```

### Subscription (`graphql-transport-ws`)

1. Connect to `ws://localhost:13131/graphql` with subprotocol
   `graphql-transport-ws`.
2. Send:

```json
{ "type": "connection_init" }
```

3. Start a subscription:

```json
{
  "id": "1",
  "type": "subscribe",
  "payload": {
    "query": "subscription { tick { ts connections } }"
  }
}
```

## GraphiQL in the Web UI

When the binary includes GraphQL, the Settings page includes a **GraphQL** tab.
At the top of that page you can enable/disable GraphQL immediately; when
enabled it embeds GraphiQL at `/graphql`.
