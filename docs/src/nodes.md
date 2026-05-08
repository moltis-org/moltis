# Multi-Node

Moltis can distribute work across multiple machines. A **node** is a remote
device that connects to your gateway and executes commands on your behalf.
This lets the AI agent run shell commands on a Linux server, query a Raspberry
Pi, or leverage a GPU machine — all from a single chat session.

## How It Works

```
┌──────────────┐    WebSocket     ┌─────────────────┐
│  Your laptop │◄────────────────►│  Moltis gateway  │
│  (browser)   │                  │  (moltis)        │
└──────────────┘                  └────────┬─────────┘
                                           │ WebSocket
                                  ┌────────▼─────────┐
                                  │  Remote machine   │
                                  │  (moltis node)    │
                                  └──────────────────┘
```

1. The gateway runs on your primary machine (or a server).
2. On the gateway, briefly enable node pairing.
3. On the remote machine, run `moltis node add` to register it with the gateway.
4. The gateway authenticates the node using **Ed25519 challenge-response** (TOFU
   model).
5. Once connected, the agent can execute commands on the node, query its
   telemetry, and discover its LLM providers.

Nodes are **stateless from the gateway's perspective** — they connect and
disconnect freely. There is no start/stop lifecycle managed by the gateway;
a node is available when its process is running and connected.

## Pairing a Node

The node generates an Ed25519 keypair on first run and
presents its public key to the gateway. The operator approves the key
fingerprint (TOFU model, same as SSH).

New pairing requests are disabled by default to prevent unauthenticated
connection spam. Open the pairing window only while adding a node.

1. On the gateway, enable pairing:
   ```bash
   moltis node pairing enable
   ```

2. On the remote machine:
   ```bash
   moltis node add --host ws://your-gateway:9090/ws --name "Build Server"
   ```
   The node prints its fingerprint and waits for approval.

3. Approve the pairing:
   - **Web UI**: Open Settings → Nodes → Pending tab, verify the fingerprint,
     click **Approve**.
   - **CLI** (headless gateways):
     ```bash
     moltis node pending                 # list pending requests
     moltis node approve <request-id>    # approve by ID
     ```

4. The gateway sends a challenge nonce, the node signs it, and authentication
   completes. The public key is pinned to this device (TOFU).

5. Disable new pairing requests:
   ```bash
   moltis node pairing disable
   ```

## Adding a Node

On the remote machine, register it as a node:

```bash
moltis node add --host ws://your-gateway:9090/ws --name "Build Server"
```

This saves the connection parameters to `~/.moltis/node.json` and installs an
OS service that starts on boot and reconnects on failure:

| Platform | Service file |
|----------|-------------|
| macOS | `~/Library/LaunchAgents/org.moltis.node.plist` |
| Linux | `~/.config/systemd/user/moltis-node.service` |

Options:

| Flag | Description | Default |
|------|-------------|---------|
| `--host` | Gateway WebSocket URL | (required) |
| `--name` | Display name shown in the UI | none |
| `--node-id` | Custom node identifier | random UUID |
| `--working-dir` | Working directory for commands | `$HOME` |
| `--timeout` | Max command timeout in seconds | `300` |
| `--foreground` | Run in the terminal instead of installing a service | off |

You can also set `MOLTIS_GATEWAY_URL` as an environment variable instead of
passing `--host`.

### Foreground mode

For debugging or one-off use, pass `--foreground` to run the node in the
current terminal session instead of installing a service:

```bash
moltis node add --host ws://your-gateway:9090/ws --name "Build Server" --foreground
```

Press `Ctrl+C` to disconnect.

## Removing a Node

To disconnect this machine and remove the background service:

```bash
moltis node remove
```

This stops the service, removes the service file, and deletes the saved
configuration from `~/.moltis/node.json`.

## Checking Status

```bash
moltis node status
```

Shows the gateway URL, display name, and whether the background service is
running.

## Node Fingerprint

```bash
moltis node fingerprint
```

Prints the Ed25519 public key fingerprint (`SHA256:<base64>`) for this node.
Use this to verify the key shown in the gateway UI during pairing.

## Logs

```bash
moltis node logs
# Tail the log:
tail -f $(moltis node logs)
```

## Selecting a Node in Chat

Once a node is connected, you can target it from a chat session:

- **UI dropdown**: The chat toolbar shows a node selector next to the model
  picker. Select a node to route all `exec` commands to it. Select "Local" to
  revert to local execution. When `tools.exec.host = "ssh"`, Moltis also shows
  either the legacy configured SSH target from `tools.exec.ssh_target` or any
  managed SSH targets you created in **Settings → SSH** as first-class
  execution options.
- **Agent tools**: The agent can call `nodes_list`, `nodes_describe`, and
  `nodes_select` to programmatically pick a node based on capabilities or
  telemetry.

The node assignment is per-session and persists across page reloads.

## Node Telemetry

Connected nodes report system telemetry every 30 seconds:

- CPU count and usage
- Memory total and available
- Disk total and available (root partition)
- System uptime
- Installed runtimes (Python, Node.js, Ruby, Go, Rust, Java)
- Available LLM providers (Ollama models, API key presence)

This data is visible on the Nodes page and available to the agent via the
`nodes_describe` tool.

If you configure `tools.exec.host = "ssh"`, the Nodes page also shows SSH
targets even though they are not WebSocket-paired nodes. This makes the active
remote execution route visible instead of hiding it in config. The UI renders
these separately from paired nodes so it is clear that SSH targets do not
report telemetry or presence.

Managed SSH targets now support:

- named labels, so session routing is readable instead of `deploy@box`
- a default target, used when a chat session does not pin a specific route
- connectivity tests from the web UI
- either **System OpenSSH** auth or a **managed deploy key**
- optional host-key pinning via a pasted `known_hosts` line
- one-click scan, refresh, and clear actions for saved host pins in Settings
- passphrase-protected private-key imports during setup

The Nodes page also includes a **Remote Exec Status** panel that acts like a
lightweight doctor:

- shows whether Moltis is currently configured for `local`, `node`, or `ssh`
- reports paired-node inventory and managed SSH inventory
- flags obvious misconfigurations, such as `tools.exec.host = "ssh"` with no
  active target or a managed key that cannot be decrypted because the vault is
  locked
- warns when the active managed SSH route is not host-pinned
- lets you pin, refresh, or clear the active managed route directly from the
  doctor panel
- lets you test the active SSH route without leaving the page

The CLI now mirrors the basic setup view with `moltis doctor`, including:

- active remote-exec backend (`local`, `node`, or `ssh`)
- SSH client discovery and version
- managed SSH key / target / host-pin inventory
- warnings for legacy `tools.exec.ssh_target` config and unpinned active routes

## CLI Reference

| Command | Description |
|---------|-------------|
| `moltis node add --host <url>` | Join this machine to a gateway as a node |
| `moltis node add ... --foreground` | Run in the terminal instead of installing a service |
| `moltis node fingerprint` | Print this node's Ed25519 fingerprint |
| `moltis node list` | List all connected nodes |
| `moltis node pairing status` | Show whether new node pairing requests are accepted |
| `moltis node pairing enable` | Enable new node pairing requests |
| `moltis node pairing disable` | Disable new node pairing requests |
| `moltis node pending` | List pending pairing requests |
| `moltis node approve <id>` | Approve a pending pairing request |
| `moltis node reject <id>` | Reject a pending pairing request |
| `moltis node remove` | Disconnect this machine and remove the service |
| `moltis node status` | Show connection info and service status |
| `moltis node logs` | Print log file path |

## Security

### Node Identity (TOFU)

Nodes authenticate using **Ed25519 challenge-response**, following the same
Trust On First Use model as SSH:

- **First connection**: The node presents its public key. The operator verifies
  the fingerprint and approves the pairing.
- **Subsequent connections**: The gateway sends a random 32-byte nonce. The node
  signs it with its private key. The gateway verifies the signature against the
  pinned public key.
- **Key pinning**: Once a public key is approved for a device, the gateway
  rejects any future connection from that device with a different key. This
  prevents impersonation.
- **Re-keying**: If a node legitimately needs a new key (e.g., after a disk
  wipe), revoke the old device from the Nodes page, then re-pair.

The private key (`~/.moltis/node_key`) is stored with mode 0600. The gateway
only stores the public key. No shared secret crosses the wire.

### General

- **Environment filtering**: When the gateway forwards commands to a node, only
  safe environment variables are forwarded (`TERM`, `LANG`, `LC_*`). Secrets
  like API keys, `DYLD_*`, and `LD_PRELOAD` are always blocked.
- **Key revocation**: Revoke from the Nodes page at any time. The node
  will be disconnected on its next reconnect attempt.
