+++
name = "dcg-guard"
description = "Blocks destructive commands using Destructive Command Guard (dcg)"
emoji = "🛡️"
events = ["BeforeToolCall"]
command = "./handler.sh"
timeout = 5
+++

# Destructive Command Guard (dcg)

Uses the external [dcg](https://github.com/Dicklesworthstone/destructive_command_guard)
tool to scan shell commands before execution. dcg ships 49+ pattern categories
covering filesystem, git, database, cloud, and infrastructure commands.

This hook is **seeded by default** into `~/.moltis/hooks/dcg-guard/` on first
run. When `dcg` is not installed the hook is a no-op (all commands pass through).

## Install dcg

```bash
cargo install dcg
```

Once installed, the hook will automatically start guarding destructive commands
on the next Moltis restart.
