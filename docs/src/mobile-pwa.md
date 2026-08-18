# Mobile PWA and Push Notifications

Moltis can be installed as a Progressive Web App (PWA) on mobile devices, providing a native app-like experience with push notifications.

## Installing on Mobile

### iOS (Safari)

1. Open moltis in Safari
2. Tap the Share button (box with arrow)
3. Scroll down and tap "Add to Home Screen"
4. Tap "Add" to confirm

The app will appear on your home screen with the moltis icon.

### Android (Chrome)

1. Open moltis in Chrome
2. You should see an install banner at the bottom - tap "Install"
3. Or tap the three-dot menu and select "Install app" or "Add to Home Screen"
4. Tap "Install" to confirm

The app will appear in your app drawer and home screen.

## PWA Features

When installed as a PWA, moltis provides:

- **Standalone mode**: Full-screen experience without browser UI
- **Offline support**: Previously loaded content remains accessible, with a dedicated offline page that reloads automatically once the connection returns
- **Fast loading**: Assets are cached locally
- **Home screen icon**: Quick access from your device's home screen
- **App shortcuts**: Long-press the icon to jump straight to Chats, Projects, or Settings
- **App badge**: The installed app icon shows an unread count while notifications
  are pending (Android, Windows, macOS Dock), including when the app is fully
  closed — the service worker badges the icon as the push arrives. Badging only
  applies to an installed app; in a plain browser tab there is no icon to draw on.
- **Safe area support**: Proper spacing for notched devices (iPhone X+)
- **Non-disruptive updates**: A new version installs in the background and takes
  over after all existing app windows close. It never reloads a hidden page or
  discards an unsent draft.

## Push Notifications

Push notifications allow you to receive alerts when the LLM responds, even when you're not actively viewing the app.

### Enabling Push Notifications

1. Open the moltis app (must be installed as PWA on Safari/iOS)
2. Go to **Settings > Notifications**
3. Click **Enable** to subscribe to push notifications
4. When prompted, allow notification permissions

**Safari/iOS Note**: Push notifications only work when the app is installed as a PWA. If you see "Installation required", add moltis to your Dock first:
- **macOS**: File → Add to Dock
- **iOS**: Share → Add to Home Screen

### Managing Subscriptions

The Settings > Notifications page shows all subscribed devices:

- **Device name**: Parsed from user agent (e.g., "Safari on macOS", "iPhone")
- **IP address**: Client IP at subscription time (supports proxies via X-Forwarded-For)
- **Subscription date**: When the device subscribed

You can remove any subscription by clicking the **Remove** button. This works
from any device and durably revokes that endpoint: automatic presence and
rotation recovery cannot silently add it back. The user must explicitly enable
notifications again on the removed device.

Subscription changes are broadcast in real-time via WebSocket, so all connected clients see updates immediately.

### How It Works

Moltis uses the Web Push API with VAPID (Voluntary Application Server Identification) keys:

1. **VAPID Keys**: On first run, the server generates a P-256 ECDSA key pair
2. **Subscription**: The browser creates a push subscription using the server's public key
3. **Registration**: The subscription details are sent to the server and stored
4. **Notification**: When you need to be notified, the server encrypts and sends a push message

Subscription endpoints must be absolute public HTTPS URLs. Moltis rejects
loopback, private, link-local, and malformed destinations before storing them
and checks them again before delivery. Sends have a ten-second deadline and
bounded concurrency, so an unavailable push service cannot hold up channel
delivery.

### Push API Routes

The gateway exposes these API endpoints for push notifications:

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/api/push/vapid-key` | GET | Get the VAPID public key for subscription |
| `/api/push/subscribe` | POST | Register a push subscription |
| `/api/push/unsubscribe` | POST | Remove a push subscription |
| `/api/push/presence` | POST | Report which session this device is viewing |
| `/api/push/test` | POST | Send a test notification to every subscribed device |
| `/api/push/status` | GET | Get push service status and subscription list |

API keys need `operator.read` for the VAPID key and `operator.write` for
subscription, presence, and test operations. Because status includes device
metadata and complete endpoints, `/api/push/status` requires `operator.admin`.
Password, passkey, and local access retain full access.

### Subscribe Request

```json
{
  "endpoint": "https://fcm.googleapis.com/fcm/send/...",
  "keys": {
    "p256dh": "base64url-encoded-key",
    "auth": "base64url-encoded-auth"
  },
  "replaces": "https://fcm.googleapis.com/fcm/send/old-endpoint",
  "revive": false
}
```

`replaces` is optional. The service worker sends it when the browser rotates a
subscription (`pushsubscriptionchange`) so the dead endpoint is retired in the
same request rather than lingering until its next delivery failure.

`revive` defaults to `false`. Automatic reconciliation always leaves it false,
so a remotely removed device stays removed. Only an explicit user **Enable**
action sends `true`; that clears the endpoint's durable revocation marker.
Registering a revoked endpoint without that authorization returns `410 Gone`.

### Presence Request

```json
{
  "endpoint": "https://fcm.googleapis.com/fcm/send/...",
  "client_id": "7ec0d5ae-...",
  "sequence": 42,
  "session_key": "main",
  "visible": true
}
```

Returns `204 No Content` when recorded, `404 Not Found` if the server merely
does not know the endpoint, or `410 Gone` if it was explicitly revoked. A `404`
replaces the unknown browser capability with a fresh subscription before
registering it. A `410` instead disables and removes the browser subscription so
remote revocation remains effective.

### Status Response

```json
{
  "enabled": true,
  "subscription_count": 2,
  "subscriptions": [
    {
      "endpoint": "https://fcm.googleapis.com/...",
      "device": "Safari on macOS",
      "ip": "192.168.1.100",
      "created_at": "2025-02-05T23:30:00Z"
    }
  ]
}
```

### Notification Payload

Push notifications include:

```json
{
  "title": "moltis",
  "body": "Rolled out to staging and the smoke tests pass.",
  "url": "/chats/main",
  "sessionKey": "main",
  "order": 42,
  "notificationId": "5f1c…",
  "timestamp": "2026-07-26T09:12:44Z"
}
```

The title is always `moltis`, so a private session label is not exposed on a
lock screen. The body is the reply with markdown syntax converted to readable
plain text. Notification previews may still be visible on a locked device;
control that exposure with the operating system's notification-preview setting.

Clicking a notification focuses an existing window — preferring one already
showing the target chat — and routes in place rather than reloading the app. If
no window is open, one is opened at the chat.

### Delivery Behaviour

Several details keep notifications from piling up or stepping on each other:

- **Per-session grouping**: Notifications are tagged per session, so one busy
  chat produces one notification rather than a wall of them. The replacement
  sets `renotify`, so it still alerts you instead of being swapped in silently,
  and its body says how many earlier messages it folded in.
- **Foreground suppression**: Each browser window maintains an ordered presence
  lease and refreshes it while focused. A device is skipped only when at least
  one window is visible, focused, and showing that exact chat. Other tabs and
  devices still get notified.
- **Server-side collapsing**: Messages carry a per-session `Topic`, so a device
  that was offline wakes to the latest message per session, not a backlog.
- **Ordered completion delivery**: Detached sends retain the assistant message
  order. A slow older send cannot replace a newer response from the same
  session.
- **Expiry**: Messages carry a 6-hour TTL. A push older than that is dropped by
  the push service rather than delivered as stale news.
- **Endpoint hygiene**: Push-service endpoints that expire are removed
  automatically. Rotated or merely server-forgotten subscriptions re-register,
  while explicitly revoked endpoints remain disabled until the user enables
  them again.
- **Badges**: The app badge is derived from every outstanding notification, not
  only the latest chat. Opening or dismissing one chat decrements the total
  without clearing notifications from other chats.

## Configuration

### Feature Flag

Push notifications are controlled by the `push-notifications` feature flag, which is enabled by default. To disable:

```toml
# In your Cargo.toml or when building
[dependencies]
moltis-gateway = { default-features = false, features = ["web-ui", "tls"] }
```

Or build without the feature:

```bash
cargo build --no-default-features --features web-ui,tls,tailscale,file-watcher
```

### Data Storage

Push notification data is stored in `push.json` in the data directory:

- **VAPID keys**: Generated once and reused
- **Subscriptions**: List of all registered browser subscriptions
- **Revocations**: Removed endpoints that automatic recovery must not recreate

The store is replaced atomically, and malformed JSON disables push initialization
instead of silently rotating VAPID keys and invalidating every subscription. The
rest of the gateway still starts. VAPID keys are persisted so subscriptions
remain valid across restarts.

## Mobile UI Considerations

The mobile interface adapts for smaller screens:

- **Navigation drawer**: The sidebar becomes a slide-out drawer on mobile
- **Sessions panel**: Displayed as a bottom sheet that can be swiped
- **Touch targets**: Minimum 44px touch targets for accessibility
- **Safe areas**: Proper insets for devices with notches or home indicators

### Responsive Breakpoints

- **Mobile**: < 768px width (drawer navigation)
- **Desktop**: ≥ 768px width (sidebar navigation)

## Browser Support

| Feature | Chrome | Safari | Firefox | Edge |
|---------|--------|--------|---------|------|
| PWA Install | ✅ | ✅ (iOS) | ❌ | ✅ |
| Push Notifications | ✅ | ✅ (iOS 16.4+) | ✅ | ✅ |
| Service Worker | ✅ | ✅ | ✅ | ✅ |
| Offline Support | ✅ | ✅ | ✅ | ✅ |

Note: iOS push notifications require iOS 16.4 or later and the app must be installed as a PWA.

## Troubleshooting

### Notifications Not Working

1. **Send a test notification**: Settings > Notifications has a **Send** button that
   pushes to every subscribed device and reports how many accepted it. This is the
   fastest way to tell a broken subscription apart from a chat that never fired one.
2. **Check permissions**: Ensure notifications are allowed in browser/OS settings
3. **Check subscription**: Go to Settings > Notifications to see if your device is listed
4. **Check server logs**: Look for `push notification` messages for delivery status
5. **Safari/iOS specific**:
   - Must be installed as PWA (Add to Dock/Home Screen)
   - iOS requires version 16.4 or later
   - The Enable button is disabled until installed as PWA
6. **Behind a proxy**: Ensure your proxy forwards `X-Forwarded-For` or `X-Real-IP` headers

### PWA Not Installing

1. **HTTPS required**: PWAs require a secure connection (or localhost)
2. **Valid manifest**: Ensure `/manifest.json` loads correctly
3. **Service worker**: Check that `/sw.js` registers without errors
4. **Clear cache**: Try clearing browser cache and reloading

### Service Worker Issues

Clear the service worker registration:

1. Open browser DevTools
2. Go to Application > Service Workers
3. Click "Unregister" on the moltis service worker
4. Reload the page
