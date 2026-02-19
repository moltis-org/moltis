+++
name = "dcg-guard"
description = "Blocks destructive commands using Destructive Command Guard (dcg)"
events = ["BeforeToolCall"]
command = "./handler.sh"
timeout = 5

[requires]
bins = ["dcg"]
+++

# Destructive Command Guard (dcg)

Uses the external [dcg](https://github.com/Dicklesworthstone/destructive_command_guard)
tool to scan shell commands before execution. dcg ships 49+ pattern categories
covering filesystem, git, database, cloud, and infrastructure commands.

## Install

```bash
cargo install dcg
```

## Setup

Copy this directory to `~/.moltis/hooks/dcg-guard/` (global) or
`<workspace>/.moltis/hooks/dcg-guard/` (project-local).

```bash
cp -r examples/hooks/dcg-guard ~/.moltis/hooks/dcg-guard
chmod +x ~/.moltis/hooks/dcg-guard/handler.sh
```

The hook activates automatically on the next Moltis restart.
