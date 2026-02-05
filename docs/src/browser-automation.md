# Browser Automation

Moltis provides full browser automation via Chrome DevTools Protocol (CDP),
enabling agents to interact with JavaScript-heavy websites, fill forms,
click buttons, and capture screenshots.

## Overview

Browser automation is useful when you need to:

- Interact with SPAs (Single Page Applications)
- Fill forms and click buttons
- Navigate sites that require JavaScript rendering
- Take screenshots of pages
- Execute JavaScript in page context
- Maintain session state across multiple interactions

For simple page content retrieval (static HTML), prefer `web_fetch` as it's
faster and more lightweight.

## Architecture

```
┌─────────────────┐     ┌─────────────────┐     ┌──────────────────┐
│   BrowserTool   │────▶│  BrowserManager │────▶│   BrowserPool    │
│   (AgentTool)   │     │   (actions)     │     │   (instances)    │
└─────────────────┘     └─────────────────┘     └──────────────────┘
                                                         │
                                                         ▼
                                                ┌──────────────────┐
                                                │  Chrome/Chromium │
                                                │     via CDP      │
                                                └──────────────────┘
```

### Components

- **BrowserTool** (`crates/tools/src/browser.rs`) - AgentTool wrapper for LLM
- **BrowserManager** (`crates/browser/src/manager.rs`) - High-level action API
- **BrowserPool** (`crates/browser/src/pool.rs`) - Chrome instance management
- **Snapshot** (`crates/browser/src/snapshot.rs`) - DOM element extraction

## Configuration

Browser automation is **enabled by default**. The browser tool runs Chrome on
the host machine (not inside the sandbox). To customize, add to your `moltis.toml`:

```toml
[tools.browser]
enabled = true              # Enable browser support
headless = true             # Run without visible window (default)
viewport_width = 1280       # Default viewport width
viewport_height = 720       # Default viewport height
max_instances = 3           # Maximum concurrent browsers
idle_timeout_secs = 300     # Close idle browsers after 5 min
navigation_timeout_ms = 30000  # Page load timeout
# chrome_path = "/path/to/chrome"  # Optional: custom Chrome path
# user_agent = "Custom UA"         # Optional: custom user agent
# chrome_args = ["--disable-extensions"]  # Optional: extra args

# Security options (see Security section below)
# sandbox = true            # Run browser in container (not yet implemented)
# allowed_domains = ["example.com", "*.trusted.org"]  # Restrict navigation
```

### Domain Restrictions

For improved security, you can restrict which domains the browser can navigate to:

```toml
[tools.browser]
allowed_domains = [
    "docs.example.com",    # Exact match
    "*.github.com",        # Wildcard: matches any subdomain
    "localhost",           # Allow localhost
]
```

When `allowed_domains` is set, any navigation to a domain not in the list will
be blocked with an error. Wildcards (`*.domain.com`) match any subdomain and
also the base domain itself.

## Tool Usage

### Actions

| Action | Description | Required Params |
|--------|-------------|-----------------|
| `navigate` | Go to a URL | `url` |
| `snapshot` | Get DOM with element refs | - |
| `screenshot` | Capture page image | `full_page` (optional) |
| `click` | Click element by ref | `ref_` |
| `type` | Type into element | `ref_`, `text` |
| `scroll` | Scroll page/element | `x`, `y`, `ref_` (optional) |
| `evaluate` | Run JavaScript | `code` |
| `wait` | Wait for element | `selector` or `ref_` |
| `get_url` | Get current URL | - |
| `get_title` | Get page title | - |
| `back` | Go back in history | - |
| `forward` | Go forward in history | - |
| `refresh` | Reload the page | - |
| `close` | Close browser session | - |

### Workflow Example

```json
// 1. Navigate to a page
{
  "action": "navigate",
  "url": "https://example.com/login"
}
// Returns: { "session_id": "browser-abc123", "url": "https://..." }

// 2. Get interactive elements
{
  "action": "snapshot",
  "session_id": "browser-abc123"
}
// Returns element refs like:
// { "elements": [
//   { "ref_": 1, "tag": "input", "role": "textbox", "placeholder": "Email" },
//   { "ref_": 2, "tag": "input", "role": "textbox", "placeholder": "Password" },
//   { "ref_": 3, "tag": "button", "role": "button", "text": "Sign In" }
// ]}

// 3. Fill in the form
{
  "action": "type",
  "session_id": "browser-abc123",
  "ref_": 1,
  "text": "user@example.com"
}

{
  "action": "type",
  "session_id": "browser-abc123",
  "ref_": 2,
  "text": "password123"
}

// 4. Click the submit button
{
  "action": "click",
  "session_id": "browser-abc123",
  "ref_": 3
}

// 5. Take a screenshot of the result
{
  "action": "screenshot",
  "session_id": "browser-abc123"
}
// Returns: { "screenshot": "data:image/png;base64,..." }
```

## Element Reference System

The snapshot action extracts interactive elements and assigns them numeric
references. This approach (inspired by [OpenClaw](https://docs.openclaw.ai))
provides:

- **Stability**: References don't break with minor page updates
- **Security**: No CSS selectors exposed to the model
- **Reliability**: Elements identified by role/content, not fragile paths

### Extracted Element Info

```json
{
  "ref_": 1,
  "tag": "button",
  "role": "button",
  "text": "Submit",
  "href": null,
  "placeholder": null,
  "value": null,
  "aria_label": "Submit form",
  "visible": true,
  "interactive": true,
  "bounds": { "x": 100, "y": 200, "width": 80, "height": 40 }
}
```

## Comparison: Browser vs Web Fetch

| Feature | `web_fetch` | `browser` |
|---------|-------------|-----------|
| Speed | Fast | Slower |
| Resources | Minimal | Chrome instance |
| JavaScript | No | Yes |
| Forms/clicks | No | Yes |
| Screenshots | No | Yes |
| Sessions | No | Yes |
| Use case | Static content | Interactive sites |

**When to use `web_fetch`:**
- Reading documentation
- Fetching API responses
- Scraping static HTML

**When to use `browser`:**
- Logging into websites
- Filling forms
- Interacting with SPAs
- Sites that require JavaScript
- Taking screenshots

## Metrics

When the `metrics` feature is enabled, the browser module records:

| Metric | Description |
|--------|-------------|
| `moltis_browser_instances_active` | Currently running browsers |
| `moltis_browser_instances_created_total` | Total browsers launched |
| `moltis_browser_instances_destroyed_total` | Total browsers closed |
| `moltis_browser_screenshots_total` | Screenshots taken |
| `moltis_browser_navigation_duration_seconds` | Page load time histogram |
| `moltis_browser_errors_total` | Errors by type |

## Browser Tool vs Sandbox

The `browser` tool runs Chrome **on the host machine**, not inside the sandbox.
This is intentional:

- The browser tool uses Chrome DevTools Protocol (CDP) for real-time interaction
- CDP requires a persistent connection to the browser process
- Running inside the sandbox would add latency and complexity

However, if agents need to run browser automation **scripts** (Puppeteer,
Playwright, Selenium) inside the sandbox, Chromium is included in the default
sandbox packages. To run a script:

```bash
# Inside sandbox (via exec tool)
chromium --headless --no-sandbox --dump-dom https://example.com
```

Or use Puppeteer/Playwright in a Node.js script executed via the `exec` tool.

## Security Considerations

### Prompt Injection Risk

**Important**: Web pages can contain content designed to manipulate LLM behavior
(prompt injection). When the browser tool returns page content to the LLM,
malicious sites could attempt to inject instructions.

**Mitigations**:

1. **Domain restrictions**: Use `allowed_domains` to limit navigation to trusted
   sites only. This is the most effective mitigation.

2. **Review returned content**: The snapshot action returns element text which
   could contain injected prompts. Be cautious with untrusted sites.

3. **Sandbox mode** (planned): Future versions will support running the browser
   in an isolated container. Set `sandbox = true` to enable when available.

### Other Security Considerations

1. **Host browser**: The browser tool runs Chrome on the host with `--no-sandbox`
   for container compatibility. For additional isolation, consider running
   Moltis itself in a container.

2. **Resource limits**: Configure `max_instances` to prevent resource exhaustion.

3. **Idle cleanup**: Browsers are automatically closed after `idle_timeout_secs`
   of inactivity.

4. **Network access**: The browser has full network access. Use firewall rules
   if you need to restrict outbound connections.

5. **Sandbox scripts**: Browser scripts running in the sandbox (via exec tool)
   inherit sandbox network restrictions (`no_network: true` by default).

## Troubleshooting

### Browser not launching

- Ensure Chrome/Chromium is installed
- Check `chrome_path` in config if using custom location
- On Linux, install dependencies: `apt-get install chromium-browser`

### Elements not found

- Use `snapshot` to see available elements
- Elements must be visible in the viewport
- Some elements may need scrolling first

### Timeouts

- Increase `navigation_timeout_ms` for slow pages
- Use `wait` action to wait for dynamic content
- Check network connectivity

### High memory usage

- Reduce `max_instances`
- Lower `idle_timeout_secs` to clean up faster
- Consider enabling headless mode if not already
