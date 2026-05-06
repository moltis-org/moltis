---
theme: default
title: Moltis, A Rust-native Claw You Can Trust
info: |
  OpenClaw Meetup Lisbon
  March 31, 2026
class: text-left
transition: slide-left
mdc: true
drawings:
  persist: false
---

<div class="rounded-[28px] border border-orange-100 bg-gradient-to-br from-orange-50 via-white to-stone-50 p-6 shadow-sm">
  <div class="flex items-start justify-between gap-6">
    <div class="inline-flex items-center gap-2 rounded-full border border-orange-200 bg-white px-4 py-2 text-sm font-medium text-gray-700">
      <span class="text-orange-600">OpenClaw Meetup Lisbon</span>
      <span class="opacity-50">•</span>
      <span>March 31, 2026</span>
    </div>
    <img src="https://www.moltis.org/favicon.svg" alt="Moltis logo" class="h-16 w-16 rounded-2xl shadow-md" />
  </div>

  <div class="mt-10 text-5xl font-semibold uppercase tracking-[0.25em] text-orange-600">
    Moltis
  </div>

  <h1 class="mt-5 max-w-4xl text-6xl leading-[1.05] font-semibold tracking-tight text-gray-950">
    One binary. Sandboxed. Yours.
  </h1>

  <div class="mt-5 max-w-4xl text-2xl leading-9 text-gray-600">
    A secure persistent agent server for real-world AI workflows.
  </div>

  <div class="mt-8 text-3xl font-semibold text-gray-900">
    Fabien Penso · @fabienpenso
  </div>

  <div class="mt-3 text-3xl leading-10 text-gray-700">
    https://www.moltis.org
  </div>
</div>

<!--
About 60 seconds.

Open with the core claim, not the feature list.
The key line is: most agent demos prove capability once, Moltis is about whether
you would trust an agent to keep running on a machine you care about.

Audience warm-up options:
- Quick show of hands, who here has already installed OpenClaw?
- Who is actually using it more than once, not just once for the demo?
- Who is using AI daily for engineering or software development?
- Who has let an agent touch a real codebase?
- Who has let an agent run shell commands on their machine?
- Who here trusts that setup enough to leave it running unattended?
- Who has tried building their own agent infrastructure instead of just using hosted tools?

Shorter version if you want to move faster:
- Who here has tried OpenClaw?
- Who here uses agents every day?
- Who here lets agents write production code?
- Who here lets them run commands?
- Who here actually trusts that setup?
-->

---

# Why Moltis exists

<div class="grid grid-cols-3 gap-6 mt-8">
  <div class="rounded-xl border border-gray-200 px-5 py-4">
    <div class="text-xl font-semibold text-orange-600">Same direction</div>
    <div class="mt-3 text-sm leading-6">
      I loved what OpenClaw unlocked. Moltis exists because I wanted to run my own version of that future.
    </div>
  </div>
  <div class="rounded-xl border border-gray-200 px-5 py-4">
    <div class="text-xl font-semibold text-orange-600">Different tradeoffs</div>
    <div class="mt-3 text-sm leading-6">
      I wanted Rust, stronger safety boundaries, and a more defense-in-depth architecture once agents get real permissions.
    </div>
  </div>
  <div class="rounded-xl border border-gray-200 px-5 py-4">
    <div class="text-xl font-semibold text-orange-600">Less friction</div>
    <div class="mt-3 text-sm leading-6">
      I wanted installation and setup to take minutes, not a side quest, so more people could actually try it.
    </div>
  </div>
</div>

<div class="mt-8 rounded-xl border border-orange-300 bg-orange-50 px-5 py-4 text-lg">
This project started from admiration, not rejection. Moltis is my opinionated Rust-native take on the same future.
</div>

<!--

The OpenClaw conference in SF had > 1,000 people who wanted to come, but only ~200 seats available. I used to live in SF and will be moving there again soon, I've never experienced such vibe since 2010. The atmosphere was insane.

About 75 seconds.

Make this personal out loud rather than on the slide.
The slide should answer the room's question:
why does this project exist at all?

Your spoken version:
- you were in San Francisco in mid January
- you were at the first OpenClaw conference with Peter / Dave Morin
- you loved the energy immediately
- you wanted your own Rust-native version badly enough to build it fast
-->

---

# Another strong opinion: installation should not be the boss fight

<div class="mt-8 text-lg leading-8">
  <ul>
    <li>At the time, people were struggling so much with installing OpenClaw that someone in San Francisco built a business around setting it up for other people on Mac minis</li>
    <li>That was a real signal to me that the appetite was huge, but the setup still had too much friction</li>
    <li>I wanted Moltis to be something you can get running quickly, without turning setup into a side quest</li>
    <li>That meant one-binary installs, Docker and Docker Compose paths, Homebrew on macOS, and binaries for Linux architectures people actually use</li>
    <li>Installing Moltis should take a few minutes at most, and on DigitalOcean it can take about a minute</li>
  </ul>
</div>

<div class="mt-10 rounded-xl border border-orange-300 bg-orange-50 px-5 py-4 text-lg">
For me, that was the cue to take installation seriously as part of the product, not just as an afterthought.
</div>

<!--
About 60 seconds.

This is a pragmatic product point.
You can say that part of the motivation was seeing how much friction there still
was in getting agents running for normal people. Moltis takes a hard stance that
installation has to be fast enough that curiosity survives the setup process.
-->

---

# The moment agents get real permissions, the architecture changes

<div class="grid grid-cols-3 gap-6 mt-8">
  <div class="rounded-xl border border-gray-200 px-5 py-4">
    <div class="text-xl font-semibold text-orange-600">Code</div>
    <div class="mt-3 text-sm leading-6">
      If an agent can edit code and run shell commands, isolation stops being optional.
    </div>
  </div>
  <div class="rounded-xl border border-gray-200 px-5 py-4">
    <div class="text-xl font-semibold text-orange-600">Secrets</div>
    <div class="mt-3 text-sm leading-6">
      If an agent can touch credentials, auth and secret handling become product features.
    </div>
  </div>
  <div class="rounded-xl border border-gray-200 px-5 py-4">
    <div class="text-xl font-semibold text-orange-600">Continuity</div>
    <div class="mt-3 text-sm leading-6">
      If the agent is persistent, memory, sessions, and recovery matter as much as model quality.
    </div>
  </div>
</div>

<div class="mt-8 rounded-xl border border-orange-300 bg-orange-50 px-5 py-4 text-lg">
That is the design center for Moltis: not just "can it do something cool?", but "can I safely let it keep running?"
</div>

<!--
About 75 seconds.

This slide is the setup. The audience already knows agents are impressive.
The pivot is: the interesting engineering question is not capability, it's
operability and trust. That lets you talk about Moltis as infrastructure rather
than as another chat wrapper.
-->

---

# What Moltis actually is

## Local-first persistent agent server

<div class="mt-6 text-lg">
  <ul>
    <li>One Rust binary between you and multiple LLM providers</li>
    <li>One agent that can meet you across web UI, API, Telegram, Discord, WhatsApp, Teams, voice</li>
    <li>Durable sessions, long-term memory, tool use, MCP, scheduling, browser automation</li>
  </ul>
</div>

<div class="mt-6 grid grid-cols-3 gap-4 text-center">
  <div class="rounded-xl border border-gray-200 px-4 py-4">
    <div class="text-2xl font-semibold text-orange-600">1,112</div>
    <div class="mt-1 text-sm opacity-80">files</div>
  </div>
  <div class="rounded-xl border border-gray-200 px-4 py-4">
    <div class="text-2xl font-semibold text-orange-600">~295K</div>
    <div class="mt-1 text-sm opacity-80">lines of code</div>
  </div>
  <div class="rounded-xl border border-gray-200 px-4 py-4">
    <div class="text-2xl font-semibold text-orange-600">~204K</div>
    <div class="mt-1 text-sm opacity-80">lines of Rust</div>
  </div>
</div>

```text
channels + UI
      |
      v
  gateway server
      |
  agent loop + tools + providers
      |
  sessions + memory + sandbox
```

<div class="mt-6 text-base opacity-80">
Same agent, same context, multiple frontends, without handing everything to a cloud relay.
</div>

<!--
About 90 seconds.

Keep this crisp. Moltis is not "an app", it is a server for your personal
agent. Emphasize persistence, multiple providers, and multiple channels.
If you want one phrase: "ChatGPT is a tab, Moltis is infrastructure."
The codebase-size moment is there to signal that this is already a real piece of
software, not a toy prototype built over a long weekend.
-->

---

# Important features

<div class="grid grid-cols-2 gap-5 mt-5">
  <div class="rounded-xl border border-gray-200 px-5 py-4">
    <div class="text-xl font-semibold text-orange-600">Core</div>
    <div class="mt-4">
      <ul>
        <li>single binary</li>
        <li>local LLM support</li>
        <li>streaming-first responses</li>
      </ul>
    </div>
  </div>
  <div class="rounded-xl border border-gray-200 px-5 py-4">
    <div class="text-xl font-semibold text-orange-600">Security</div>
    <div class="mt-4">
      <ul>
        <li>passwords, passkeys, tokens</li>
        <li>encrypted vault</li>
        <li>filesystem or sandbox isolation</li>
      </ul>
    </div>
  </div>
  <div class="rounded-xl border border-gray-200 px-5 py-4">
    <div class="text-xl font-semibold text-orange-600">Memory</div>
    <div class="mt-4">
      <ul>
        <li>long-term memory</li>
        <li>hybrid vector + full-text search</li>
        <li>session recall and branching</li>
      </ul>
    </div>
  </div>
  <div class="rounded-xl border border-gray-200 px-5 py-4">
    <div class="text-xl font-semibold text-orange-600">Channels</div>
    <div class="mt-4">
      <ul>
        <li>web UI</li>
        <li>Telegram, Discord, WhatsApp, Teams</li>
        <li>voice input and output</li>
      </ul>
    </div>
  </div>
</div>

<div class="mt-5 text-base">
Also built in: skills, hooks, MCP, browser automation, OpenClaw import, GraphQL, scheduling, and observability.
</div>

<!--
About 75 seconds.

This slide is based on the current features page on moltis.org.
Do not read every bullet. Use it as a scan slide:
single binary, secure auth, memory, tooling, channels, migration, ops.

If the room feels technical, point at MCP, GraphQL, and session branching.
If the room feels more practical, point at install, channels, voice, and OpenClaw import.
-->

---

# Design principles

<div class="grid grid-cols-3 gap-6 mt-8">
  <div class="rounded-xl border border-gray-200 px-5 py-4">
    <div class="text-xl font-semibold text-orange-600">Security</div>
    <div class="mt-4">
      <ul>
        <li>sandboxed execution</li>
        <li>password + passkeys</li>
        <li>encrypted vault for secrets</li>
      </ul>
    </div>
  </div>
  <div class="rounded-xl border border-gray-200 px-5 py-4">
    <div class="text-xl font-semibold text-orange-600">Persistence</div>
    <div class="mt-4">
      <ul>
        <li>cross-session recall</li>
        <li>long-term memory</li>
        <li>automatic checkpoints</li>
      </ul>
    </div>
  </div>
  <div class="rounded-xl border border-gray-200 px-5 py-4">
    <div class="text-xl font-semibold text-orange-600">Ownership</div>
    <div class="mt-4">
      <ul>
        <li>your hardware</li>
        <li>one binary</li>
        <li>local-first by default</li>
      </ul>
    </div>
  </div>
</div>

<div class="mt-10 text-lg">
The goal is not just to make agents more capable. It is to make them survivable.
</div>

<!--
About 90 seconds.

This is the thesis slide. Avoid reading every bullet.
Talk through the three buckets:
security means real isolation and real auth,
persistence means the agent can have continuity without dumping raw history,
ownership means you can run it on your own hardware with understandable tradeoffs.
-->

---

# Security is part of the product

<div class="grid grid-cols-3 gap-6 mt-8">
  <div class="rounded-xl border border-gray-200 px-5 py-4">
    <div class="text-xl font-semibold text-orange-600">Safer foundation</div>
    <div class="mt-4">
      <ul>
        <li>Rust for memory safety and stronger boundaries</li>
        <li>one binary, smaller runtime surface</li>
        <li>different trust model from the start</li>
      </ul>
    </div>
  </div>
  <div class="rounded-xl border border-gray-200 px-5 py-4">
    <div class="text-xl font-semibold text-orange-600">Defense in depth</div>
    <div class="mt-4">
      <ul>
        <li>passkeys and passwords in the web UI</li>
        <li>sandboxed tool execution</li>
        <li>encrypted vault and network protections</li>
      </ul>
    </div>
  </div>
  <div class="rounded-xl border border-gray-200 px-5 py-4">
    <div class="text-xl font-semibold text-orange-600">Safer access</div>
    <div class="mt-4">
      <ul>
        <li>designed to keep strangers out</li>
        <li>Tailscale support for private access paths</li>
        <li>local-first deployment instead of exposing everything by default</li>
      </ul>
    </div>
  </div>
</div>

<div class="mt-10 rounded-xl border border-orange-300 bg-orange-50 px-5 py-4 text-lg">
One of my motivations for Moltis was simple: if agents are going to get real permissions, security cannot be a bolt-on.
</div>

<!--
About 75 seconds.

Keep this respectful to OpenClaw and focused on your motivation.
The line is not "others are insecure." The line is:
"Once agents can touch code, files, credentials, and networks, I wanted a more
defense-in-depth architecture, and Rust was part of that choice."

Mention passkeys/passwords, sandboxing, vault, and Tailscale as concrete signals
that security is part of the product shape, not just a README section.
-->

---

# What people actually use Moltis for

<div class="grid grid-cols-2 gap-5 mt-6">
  <div class="rounded-xl border border-gray-200 px-4 py-3">
    <div class="text-lg font-semibold text-orange-600">Engineering copilot</div>
    <div class="mt-2 text-sm">
      <ul>
        <li>code editing</li>
        <li>sandboxed shell</li>
        <li>session recall</li>
      </ul>
    </div>
  </div>
  <div class="rounded-xl border border-gray-200 px-4 py-3">
    <div class="text-lg font-semibold text-orange-600">Persistent personal agent</div>
    <div class="mt-2 text-sm">
      <ul>
        <li>long-term memory</li>
        <li>cross-session recall</li>
        <li>scheduled jobs</li>
      </ul>
    </div>
  </div>
  <div class="rounded-xl border border-gray-200 px-4 py-3">
    <div class="text-lg font-semibold text-orange-600">Multi-channel assistant</div>
    <div class="mt-2 text-sm">
      <ul>
        <li>web UI</li>
        <li>Telegram or Discord</li>
        <li>voice access</li>
      </ul>
    </div>
  </div>
  <div class="rounded-xl border border-gray-200 px-4 py-3">
    <div class="text-lg font-semibold text-orange-600">Platform for builders</div>
    <div class="mt-2 text-sm">
      <ul>
        <li>MCP servers</li>
        <li>custom providers</li>
        <li>GraphQL and API</li>
      </ul>
    </div>
  </div>
</div>

<div class="mt-8 rounded-xl border border-orange-300 bg-orange-50 px-5 py-4 text-lg">
The pattern is clear: people are wiring Moltis into real workflows, not treating it like a toy chatbot.
</div>

<!--
About 90 seconds.

This slide should feel concrete.
The recent GitHub activity points to real usage around:
- Telegram thread isolation and replies
- Matrix integration
- voice provider configuration
- session recall and skill portability
- managed SSH runtime UX
- custom providers and search providers
- GraphQL and MCP extensions

So the point is not hypothetical use cases. It is what people are already
trying to make work in the real world.
-->

---

# Momentum

<div class="grid grid-cols-2 gap-6 mt-10 text-center">
  <div class="rounded-xl border border-gray-200 px-5 py-6">
    <div class="text-4xl font-semibold text-orange-600">2,411</div>
    <div class="mt-2 text-base opacity-80">⭐️ GitHub stars</div>
  </div>
  <div class="rounded-xl border border-gray-200 px-5 py-6">
    <div class="text-4xl font-semibold text-orange-600">117</div>
    <div class="mt-2 text-base opacity-80">PRs merged in March</div>
  </div>
  <div class="rounded-xl border border-gray-200 px-5 py-6">
    <div class="text-4xl font-semibold text-orange-600">92</div>
    <div class="mt-2 text-base opacity-80">issues created in March</div>
  </div>
  <div class="rounded-xl border border-gray-200 px-5 py-6">
    <div class="text-4xl font-semibold text-orange-600">102</div>
    <div class="mt-2 text-base opacity-80">issues closed in March</div>
  </div>
</div>

<div class="mt-10 rounded-xl border border-orange-300 bg-orange-50 px-5 py-4 text-lg">
This is moving fast because people are actually using it, reporting breakage, and pushing it forward.
</div>

<!--
About 45 seconds.

Numbers gathered on March 30, 2026.
Month range used here is March 1, 2026 through March 30, 2026.

Stats:
- 2,411 GitHub stars
- 117 PRs merged
- 92 issues created
- 102 issues closed

This is not a vanity slide. The point is product velocity and real usage.
-->

---

# For the Rust engineers

<div class="grid grid-cols-2 gap-4 mt-3">
  <div class="rounded-xl border border-gray-200 px-5 py-3">
    <div class="text-xl font-semibold text-orange-600">Architecture</div>
    <div class="mt-2 text-sm">
      <ul>
        <li>one Cargo workspace, split into many focused sub-crates</li>
        <li>shared dependencies live in <code>[workspace.dependencies]</code></li>
        <li>clean service boundaries via traits and crate separation</li>
      </ul>
    </div>
  </div>
  <div class="rounded-xl border border-gray-200 px-5 py-3">
    <div class="text-xl font-semibold text-orange-600">Feature-gated builds</div>
    <div class="mt-2 text-sm">
      <ul>
        <li><code>web-ui</code>, <code>voice</code>, <code>vault</code></li>
        <li><code>graphql</code>, <code>local-llm</code>, <code>tailscale</code></li>
        <li><code>lightweight</code> for smaller builds</li>
      </ul>
    </div>
  </div>
  <div class="rounded-xl border border-gray-200 px-5 py-3">
    <div class="text-xl font-semibold text-orange-600">Why Rust helps here</div>
    <div class="mt-2 text-sm">
      <ul>
        <li>types carry config, protocol, and service boundaries</li>
        <li>compile-time gating beats runtime roulette</li>
        <li><code>secrecy::Secret</code> for credentials, not vibes</li>
      </ul>
    </div>
  </div>
  <div class="rounded-xl border border-gray-200 px-5 py-3">
    <div class="text-xl font-semibold text-orange-600">House rules</div>
    <div class="mt-2 text-sm">
      <ul>
        <li><code>unsafe</code> denied workspace-wide in core</li>
        <li><code>unwrap</code> and <code>expect</code> denied by lint policy</li>
        <li>tracing and metrics are first-class, crate by crate</li>
      </ul>
    </div>
  </div>
</div>

<div class="mt-4 rounded-xl border border-orange-300 bg-orange-50 px-5 py-3 text-base">
The pitch for Rust people is simple: Moltis uses Rust as an architecture tool, not just as an implementation language.
</div>

<!--
About 75 seconds.

This slide is for the people in the room who care how the thing is built.
Do not over-explain it. Hit the shape:
workspace, crate boundaries, feature flags, typed interfaces, lint policy,
and the fact that observability is built into the crates instead of bolted on.

If you need a one-liner:
"A lot of agent projects use Rust for speed. I wanted Rust for architecture,
type boundaries, and a smaller trust surface."
-->

---

# Why try Moltis if you already know OpenClaw?

<div class="grid grid-cols-3 gap-6 mt-8">
  <div class="rounded-xl border border-gray-200 px-5 py-4">
    <div class="text-xl font-semibold text-orange-600">Keep your history</div>
    <div class="mt-4">
      <ul>
        <li>read-only OpenClaw import</li>
        <li>sessions, skills, memory, providers, channels</li>
        <li>run side by side if you want</li>
      </ul>
    </div>
  </div>
  <div class="rounded-xl border border-gray-200 px-5 py-4">
    <div class="text-xl font-semibold text-orange-600">Change the tradeoffs</div>
    <div class="mt-4">
      <ul>
        <li>Rust-native architecture</li>
        <li>stronger local-first and security posture</li>
        <li>different assumptions around sandboxing and persistence</li>
      </ul>
    </div>
  </div>
  <div class="rounded-xl border border-gray-200 px-5 py-4">
    <div class="text-xl font-semibold text-orange-600">Get started faster</div>
    <div class="mt-4">
      <ul>
        <li>simple install paths</li>
        <li>one-binary mindset</li>
        <li>low migration risk, low setup friction</li>
      </ul>
    </div>
  </div>
</div>

<div class="mt-10 rounded-xl border border-orange-300 bg-orange-50 px-5 py-4 text-lg">
The ask is small: keep what already works for you, and try a different trust model without starting over.
</div>

<!--
About 75 seconds.

This is much better when framed as a practical trial, not as a tribal choice.
The room does not need a justification slide. They need a reason to care:
you can preserve your existing workspace, try different architecture tradeoffs,
and do it without a migration cliff.
-->

---

# If I skip the live demo, this is the product arc

<div class="grid grid-cols-3 gap-4 mt-6">
  <div>
    <img src="https://www.moltis.org/screenshots/1.png" alt="Secure onboarding" class="rounded-lg shadow-md max-h-72 mx-auto" />
    <div class="mt-3 text-sm text-center opacity-80">Secure onboarding, password or passkey</div>
  </div>
  <div>
    <img src="https://www.moltis.org/screenshots/4.png" alt="Chat with tool use" class="rounded-lg shadow-md max-h-72 mx-auto" />
    <div class="mt-3 text-sm text-center opacity-80">Tool-using chat, voice, maps, live interaction</div>
  </div>
  <div>
    <img src="https://www.moltis.org/screenshots/5.png" alt="Sandboxed execution" class="rounded-lg shadow-md max-h-72 mx-auto" />
    <div class="mt-3 text-sm text-center opacity-80">Shell execution inside the sandbox, not on the host</div>
  </div>
</div>

<div class="mt-8 text-lg">
That is the story: secure setup, persistent interaction, sandboxed action.
</div>

<!--
About 90 seconds.

This is the backup-demo slide and it can also be your primary demo.
Walk left to right:
1. secure setup and identity,
2. real interaction with tools,
3. execution in the sandbox rather than on the host.

If time is tight, spend only 45 seconds here.
-->

---

# One core, more places to use it

<div class="grid grid-cols-2 gap-8 mt-8">
  <div class="rounded-xl border border-gray-200 px-5 py-4">
    <div class="text-xl font-semibold text-orange-600">Mobile</div>
    <div class="mt-4">
      <ul>
        <li>native iPhone access to the same agent</li>
        <li>better mobile presence and notifications</li>
        <li>same sessions when you are away from your desk</li>
      </ul>
    </div>
  </div>
  <div class="rounded-xl border border-gray-200 px-5 py-4">
    <div class="text-xl font-semibold text-orange-600">Desktop</div>
    <div class="mt-4">
      <ul>
        <li>native macOS app experience</li>
        <li>same Moltis core reused through the Rust Swift bridge</li>
        <li>native UI without splitting the product into separate engines</li>
      </ul>
    </div>
  </div>
</div>

<div class="mt-10 rounded-xl border border-orange-300 bg-orange-50 px-5 py-4 text-lg">
The point is not "more apps." The point is the same local-first agent becoming available everywhere you actually are.
</div>

<!--
About 60 seconds.

This is better framed as user value, not roadmap.
The interesting point is that native mobile and desktop frontends are ways to
reach the same agent and same sessions, not separate products with separate
logic.
-->

---

# Takeaways

<div class="mt-8 text-2xl leading-10">

1. Agents are becoming infrastructure, not toys
2. The trust model is part of the product
3. Moltis is one answer: local, persistent, auditable

</div>

<div class="mt-12 text-lg">
If you want an agent server you can actually leave running, come find me after.
</div>

<div class="mt-10 text-3xl leading-[1.6] font-semibold text-center">
🔗 https://www.moltis.org
</div>

<div class="mt-4 text-xl leading-9 font-medium text-center">
🐙 https://github.com/moltis-org/moltis &nbsp;&nbsp;&nbsp;•&nbsp;&nbsp;&nbsp; 📚 https://docs.moltis.org
</div>

<!--
About 60 seconds.

End on the idea that the architecture choices are the product.
Do not end on a laundry list of features.
If there is applause, stop. If there are 20 spare seconds, point people to the
OpenClaw import path and invite them to try it without migration risk.
-->

---

# Appendix, if questions go technical

### Short answers

- Why Rust: memory safety, one binary, auditable boundaries
- Why local-first: keys, memory, and sessions stay under your control
- Why not just plugins everywhere: supply-chain risk and harder auditing
- Why persistence matters: useful agents need memory, checkpoints, and continuity
- Why the OpenClaw import matters: migration without cliff-edge risk

<!--
Use only if questions start immediately or you need a backup final slide.
-->
