# Agent Identity & Onboarding Protocols

Moltis provides personal agent servers. The Identity Protocol (L2) and Onboarding Protocol (L1) standardize how personal agents declare who they are and prove they're ready for work — enabling cross-server trust without a central authority.

## Identity Protocol (L2)

Each agent gets an Ed25519 keypair. The public key is the agent's permanent, verifiable identifier across Moltis servers.

```python
from works_with_agents import IdentityProtocol

identity = IdentityProtocol.create_agent("my-moltis-agent")
# identity.public_key   → Ed25519 public key
# identity.fingerprint  → stable hash for identification
```

### What Identity Enables

| Without Identity | With Identity |
|-----------------|---------------|
| Agent identified by server:port | Cryptographic keypair |
| "Trust whoever connects" | Verify before accepting work |
| No audit trail | Signed, immutable record |
| Name collisions possible | Unique Ed25519 fingerprints |

## Onboarding Protocol (L1)

Before an agent is trusted with work, it goes through structured onboarding. An authority (another Moltis agent, or a trusted server) issues a signed certificate listing verified capabilities.

```json
{
  "agent_id": "my-moltis-agent",
  "authority": "onboarding-server",
  "capabilities": ["file_management", "scheduled_tasks", "web_search"],
  "issued_at": "2026-05-06T19:00:00Z",
  "signature": "ed25519:..."
}
```

### Why It Matters for Moltis

Personal agent servers are powerful but isolated. With Identity + Onboarding:

1. **Your agents can discover each other** across servers
2. **Delegation is verifiable** — Agent A signs handoffs to Agent B
3. **New agents prove themselves** before getting access to shared resources
4. **Fleet management at scale** — every agent has a verifiable record

## Getting Started

Both protocols are open source (CC BY 4.0):

- **Identity spec:** https://workswithagents.com/specs/identity.md
- **Onboarding spec:** https://workswithagents.com/specs/onboarding.md
- **Python SDK:** `pip install works-with-agents`
- **Reference implementations:** 6 languages (Python, TypeScript, Go, C#, Rust, Shell)

## Related Specs

- [Handoff Protocol](https://workswithagents.com/specs/handoff.md) — Signed task transfer between agents
- [Capability Manifest](https://workswithagents.com/specs/capability-manifest.md) — Declarative capability listing
- [Trust Score](https://workswithagents.com/specs/trust-score.md) — Verifiable agent reputation
