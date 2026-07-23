# Service Management

Moltis can be installed as an OS service so it starts automatically on boot
and restarts after crashes.

## Install

```bash
moltis service install
```

This creates a service definition and starts it immediately:

| Platform | Service file | Init system |
|----------|-------------|-------------|
| macOS | `~/Library/LaunchAgents/org.moltis.gateway.plist` | launchd (user agent) |
| Linux | `~/.config/systemd/user/moltis.service` | systemd (user unit) |
| Linux containers without systemd | `~/.moltis/moltis-service-supervisor.sh` | portable user supervisor |

The launchd and systemd configurations:

- **Start on boot** (`RunAtLoad` / `WantedBy=default.target`)
- **Restart on failure** with a 10-second cooldown
- **Log to** `~/.moltis/moltis.log`

On Linux, Moltis first uses `systemd --user` when it is available. Some
development containers, including Coder/devbox environments, do not run systemd
or provide the SysV `service` command. In those environments, `moltis service
install` falls back to a small user-owned supervisor script in `~/.moltis/`.
The fallback starts Moltis in the background, records pid files, supports
`status`, `stop`, `restart`, and restarts Moltis after crashes.

The portable supervisor is tied to the current container session. If the
workspace container is recreated, configure your devbox/Coder startup hook to
run `moltis service restart` after installing Moltis.

### Options

You can pass `--bind`, `--port`, and `--log-level` to bake them into the
service definition:

```bash
moltis service install --bind 0.0.0.0 --port 8080 --log-level debug
```

These flags are written into the service file. The service reads the rest of
its configuration from `~/.moltis/moltis.toml` as usual.

## Manage

```bash
moltis service status     # Show running/stopped/not-installed and PID
moltis service stop       # Stop the service
moltis service restart    # Restart the service
moltis service logs       # Print the log file path
```

To tail the logs:

```bash
tail -f $(moltis service logs)
```

## Uninstall

```bash
moltis service uninstall
```

This stops the service, removes the service file, and cleans up.

## CLI Reference

| Command | Description |
|---------|-------------|
| `moltis service install` | Install and start the service |
| `moltis service uninstall` | Stop and remove the service |
| `moltis service status` | Show service status and PID |
| `moltis service stop` | Stop the service |
| `moltis service restart` | Restart the service |
| `moltis service logs` | Print log file path |

## How It Differs from `moltis node add`

`moltis service install` manages the **gateway** — the main Moltis server
that hosts the web UI, chat sessions, and API.

`moltis node add` registers a **headless node** — a client process on a
remote machine that connects back to a gateway for command execution. See
[Multi-Node](nodes.md) for details.

| | `moltis service` | `moltis node` |
|---|---|---|
| What it runs | The gateway server | A node client |
| Needs `--host`/`--token` | No | Yes |
| Config source | `~/.moltis/moltis.toml` | `~/.moltis/node.json` |
| launchd label | `org.moltis.gateway` | `org.moltis.node` |
| systemd unit | `moltis.service` | `moltis-node.service` |
