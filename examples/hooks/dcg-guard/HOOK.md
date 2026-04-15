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

Pin to a released tag and verify the script's SHA-256 before executing it —
never pipe an unpinned `curl | bash` from `main`. Check the project's
[releases page](https://github.com/Dicklesworthstone/destructive_command_guard/releases)
for the latest tag and expected checksum.

```bash
DCG_VERSION="v0.4.0"
DCG_SHA256="2cd1287c30cc7bfca3ec6e45a3a474e9bb8f8586dfe83d78db0d6c3a25f3b55c"
curl -fsSL "https://raw.githubusercontent.com/Dicklesworthstone/destructive_command_guard/${DCG_VERSION}/install.sh" -o /tmp/dcg-install.sh
echo "${DCG_SHA256}  /tmp/dcg-install.sh" | shasum -a 256 -c - && bash /tmp/dcg-install.sh
rm /tmp/dcg-install.sh
```

Once installed, the hook will automatically start guarding destructive commands
on the next Moltis restart.
