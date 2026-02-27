# Moltis Courier

Privacy-preserving APNS push relay for self-hosted Moltis gateways.

Courier holds a single Apple Push Notification Service (APNS) key and forwards
opaque silent "wake up" pushes (`content-available: 1`) on behalf of gateways.
The relay never sees message text or metadata — the iOS app reconnects to its
own gateway to fetch actual content.

## Getting a .p8 key

1. Sign in to [App Store Connect](https://appstoreconnect.apple.com/)
2. Go to **Users and Access → Integrations → Keys**
3. Create a new key with **Apple Push Notifications service (APNs)** enabled
4. Download the `.p8` file (you can only download it once)
5. Note the **Key ID** and your **Team ID**

## Running

```bash
moltis-courier \
  --key-path /path/to/AuthKey_XXXXXXXXXX.p8 \
  --key-id XXXXXXXXXX \
  --team-id YYYYYYYYYY \
  --bundle-id org.moltis.app \
  --auth-token "your-shared-secret"
```

### Options

| Flag           | Default       | Description                            |
|----------------|---------------|----------------------------------------|
| `--bind`       | `0.0.0.0`     | Address to bind                        |
| `--port`       | `8090`        | Port to listen on                      |
| `--key-path`   | *(required)*  | Path to the `.p8` private key file     |
| `--key-id`     | *(required)*  | Apple key identifier                   |
| `--team-id`    | *(required)*  | Apple team identifier                  |
| `--bundle-id`  | *(required)*  | iOS app bundle identifier (apns-topic) |
| `--auth-token` | *(optional)*  | Shared secret for gateway auth         |

## API

### `POST /push`

Send a silent push notification to a device.

```bash
curl -X POST http://localhost:8090/push \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer your-shared-secret" \
  -d '{"device_token": "hex-encoded-apns-token", "environment": "production"}'
```

**Request body:**

| Field          | Type   | Default        | Description                    |
|----------------|--------|----------------|--------------------------------|
| `device_token` | string | *(required)*   | Hex-encoded APNS device token  |
| `environment`  | string | `"production"` | `"production"` or `"sandbox"` |

**Responses:**

- `200` — `{"status": "sent"}`
- `400` — `{"error": "missing device_token"}`
- `401` — `{"error": "unauthorized"}`
- `502` — `{"error": "apns rejected: ..."}`

### `GET /health`

Returns `200 OK` with an empty body.
