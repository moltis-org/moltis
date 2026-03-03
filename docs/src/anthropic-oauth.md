# Why Moltis Doesn't Support Anthropic OAuth

A common request is browser-based OAuth login for Anthropic, similar to what
Moltis offers for OpenAI Codex and GitHub Copilot.  This page explains why that
isn't possible and what to do instead.

## TL;DR

Anthropic **does not offer an OAuth program for third-party tools**.  Their
OAuth flow is locked to Claude Code and Claude.ai only.  The only supported way
to use Anthropic models in Moltis is with an API key from the
[Anthropic Console](https://console.anthropic.com).

## Background

Claude Code (Anthropic's CLI) authenticates via an OAuth 2.0 PKCE flow against
`console.anthropic.com`.  The temporary OAuth token is then exchanged for a
permanent API key through an internal endpoint
(`/api/oauth/claude_cli/create_api_key`).  The path segment `/claude_cli/`
signals this is a **CLI-specific, internal endpoint** — not a public API.

The client ID used by Claude Code (`9d1c250a-…`) is hard-coded for that single
application.  Anthropic does not provide a way to register new client IDs for
third-party projects.

## Anthropic's Policy

In February 2026 Anthropic updated their
[Legal and Compliance](https://www.anthropic.com/legal) page with an explicit
restriction:

> OAuth authentication (used with Free, Pro, and Max plans) is intended
> exclusively for Claude Code and Claude.ai.  Using OAuth tokens obtained
> through Claude Free, Pro, or Max accounts in any other product, tool, or
> service — including the Agent SDK — is not permitted and constitutes a
> violation of the Consumer Terms of Service.

This isn't just a policy statement — Anthropic deployed **server-side
enforcement** in January 2026.  OAuth tokens from consumer plans now return
errors outside of Claude Code and Claude.ai:

> *"This credential is only authorized for use with Claude Code and cannot be
> used for other API requests."*

Several projects that attempted this approach — including
[Auto-Claude](https://github.com/AndyMik90/Auto-Claude/issues/1871),
[Goose](https://github.com/block/goose/issues/3647), and
[OpenCode](https://github.com/anthropics/claude-code/issues/28091) — were
forced to drop OAuth and switch to standard API keys.

## What About Reusing the Claude Code Client ID?

Even if the OAuth flow and key creation technically succeed today, using Claude
Code's client ID from a different application:

- **Violates the Consumer Terms of Service** (Section 3.7 — no automated
  access through bots or scripts except via official API keys).
- **Risks key revocation** — Anthropic reserves the right to revoke credentials
  *without prior notice*.
- **Could break at any time** — the client ID and internal endpoints are not
  part of a public, stable API surface.

## How to Use Anthropic in Moltis

1. Go to [console.anthropic.com](https://console.anthropic.com) and create an
   account (or sign in).
2. Navigate to **Settings → API Keys** and create a new key.
3. In Moltis, go to **Settings → Providers → Anthropic** and paste the key.

Alternatively, set the `ANTHROPIC_API_KEY` environment variable or add it to
your `moltis.toml`:

```toml
[providers.anthropic]
enabled = true
api_key = "sk-ant-api03-..."
```

This is the method Anthropic
[officially recommends](https://docs.anthropic.com/en/api/getting-started) for
all third-party integrations.

## Will This Change?

If Anthropic introduces a developer OAuth program with client ID registration
in the future, Moltis will adopt it.  The generic infrastructure for
OAuth-to-API-key exchange already exists in the codebase (the `api_key_endpoint`
field on `OAuthConfig`), so adding support would be straightforward.

## References

- [Anthropic API — Getting Started](https://docs.anthropic.com/en/api/getting-started) — official auth docs (API key only)
- [The Register — Anthropic clarifies ban on third-party tool access](https://www.theregister.com/2026/02/20/anthropic_clarifies_ban_third_party_claude_access/)
- [HN — Anthropic officially bans subscription auth for third-party use](https://news.ycombinator.com/item?id=47069299)
- [Claude Code #28091 — Anthropic disabled OAuth tokens for third-party apps](https://github.com/anthropics/claude-code/issues/28091)
- [Auto-Claude #1871 — OAuth policy violation](https://github.com/AndyMik90/Auto-Claude/issues/1871)
- [Goose #3647 — Anthropic OAuth for third-party users](https://github.com/block/goose/issues/3647)
