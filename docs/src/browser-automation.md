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

Browser automation is **enabled by default**. To customize, add to your `moltis.toml`:

```toml
[tools.browser]
enabled = true              # Enable browser support
headless = true             # Run without visible window (default)
viewport_width = 2560       # Default viewport width
viewport_height = 1440      # Default viewport height
device_scale_factor = 2.0   # HiDPI/Retina scaling (1.0 = standard, 2.0 = Retina)

# Pool management
max_instances = 0           # 0 = unlimited (limited by memory), >0 = hard limit
memory_limit_percent = 90   # Block new instances when memory exceeds this %
idle_timeout_secs = 300     # Close idle browsers after 5 min
navigation_timeout_ms = 30000  # Page load timeout

# Optional customization
# chrome_path = "/path/to/chrome"  # Custom Chrome path
# user_agent = "Custom UA"         # Custom user agent
# chrome_args = ["--disable-extensions"]  # Extra args

# Sandbox image (browser sandbox mode follows session sandbox mode)
sandbox_image = "browserless/chrome"  # Container image for sandboxed sessions
# allowed_domains = ["example.com", "*.trusted.org"]  # Restrict navigation
```

### Memory-Based Pool Limits

By default, browser instances are limited by system memory rather than a fixed count:

- `max_instances = 0` (default): Unlimited instances, blocked only when memory
  exceeds `memory_limit_percent`
- `memory_limit_percent = 90`: New browsers blocked when system memory > 90%
- Set `max_instances > 0` for a hard limit if you prefer fixed constraints

This allows multiple chat sessions to each have their own browser without
artificial limits, while protecting system stability when memory is constrained.

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

### Automatic Session Tracking

The browser tool automatically tracks and reuses session IDs. After a `navigate`
action creates a session, subsequent actions will reuse it without needing to
pass `session_id` explicitly:

```json
// 1. Navigate (creates session)
{ "action": "navigate", "url": "https://example.com" }

// 2. Snapshot (session_id auto-injected)
{ "action": "snapshot" }

// 3. Click (session_id auto-injected)
{ "action": "click", "ref_": 1 }

// 4. Screenshot (session_id auto-injected)
{ "action": "screenshot" }

// 5. Close (clears tracked session)
{ "action": "close" }
```

This prevents pool exhaustion from LLMs that forget to pass the session_id.

### Workflow Example

```json
// 1. Navigate to a page
{
  "action": "navigate",
  "url": "https://example.com/login"
}
// Returns: { "session_id": "browser-abc123", "url": "https://..." }

// 2. Get interactive elements (session_id optional - auto-tracked)
{
  "action": "snapshot"
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
  "ref_": 1,
  "text": "user@example.com"
}

{
  "action": "type",
  "ref_": 2,
  "text": "password123"
}

// 4. Click the submit button
{
  "action": "click",
  "ref_": 3
}

// 5. Take a screenshot of the result
{
  "action": "screenshot"
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

## Sandbox Mode

Browser sandbox mode **automatically follows the session's sandbox mode**. When
a chat session uses sandbox mode (controlled by `[tools.exec.sandbox]`), the
browser tool will also run in a sandboxed container. When the session is not
sandboxed, the browser runs directly on the host.

### Host Mode

When the session is not sandboxed (or sandbox mode is "off"), Chrome runs
directly on the host machine. This is faster but the browser has full access
to the host network and filesystem.

### Sandbox Mode

When the session is sandboxed, Chrome runs inside a Docker container with:

- **Network isolation**: Browser can access the internet but not local services
- **Filesystem isolation**: No access to host filesystem
- **Automatic lifecycle**: Container started/stopped with browser session
- **Readiness detection**: Waits for Chrome to be fully ready before connecting

```toml
[tools.browser]
sandbox_image = "browserless/chrome"  # Container image for sandboxed sessions
```

Requirements:
- Docker or Apple Container must be installed and running
- The container image is pulled automatically on first use
- Session sandbox mode must be enabled (`[tools.exec.sandbox] mode = "all"`)

### Exec Tool Scripts

If agents need to run browser automation **scripts** (Puppeteer, Playwright,
Selenium) inside the command sandbox, Chromium is included in the default
sandbox packages:

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

3. **Sandbox mode**: Use sandboxed sessions to run the browser in an isolated
   Docker container for additional security. Browser sandbox follows session
   sandbox mode automatically.

### Other Security Considerations

1. **Host vs Sandbox mode**: Browser sandbox mode follows the session's sandbox
   mode. For sandboxed sessions, the browser runs in a Docker container with
   network/filesystem isolation. For non-sandboxed sessions, the browser runs
   on the host with `--no-sandbox` for container compatibility.

2. **Resource limits**: Browser instances are limited by memory usage (default:
   block when > 90% used). Set `max_instances > 0` for a hard limit.

3. **Idle cleanup**: Browsers are automatically closed after `idle_timeout_secs`
   of inactivity.

4. **Network access**: In host mode, the browser has full network access. In
   sandbox mode, the browser can reach the internet but not local services.
   Use firewall rules for additional restrictions.

5. **Sandbox scripts**: Browser scripts running in the exec sandbox (Puppeteer,
   Playwright) inherit sandbox network restrictions (`no_network: true` by default).

## Browser Detection

Moltis automatically detects installed Chromium-based browsers in the following order:

1. **Custom path** from `chrome_path` config
2. **CHROME environment variable**
3. **Platform-specific app bundles** (macOS/Windows)
   - macOS: `/Applications/Google Chrome.app`, `/Applications/Chromium.app`, etc.
   - Windows: `C:\Program Files\Google\Chrome\Application\chrome.exe`, etc.
4. **PATH executables** (fallback): `chrome`, `chromium`, `msedge`, `brave`, etc.

If no browser is found, Moltis displays platform-specific installation instructions.

### Supported Browsers

Any Chromium-based browser works:
- Google Chrome
- Chromium
- Microsoft Edge
- Brave
- Opera
- Vivaldi
- Arc (macOS)

## Screenshot Display

When the browser tool takes a screenshot, it's displayed in the chat UI:

- **Thumbnail view**: Screenshots appear as clickable thumbnails (200×150px max)
- **Fullscreen lightbox**: Click to view full-size with dark overlay
- **Scrollable view**: Long screenshots can be scrolled within the lightbox
- **Download button**: Save screenshot to disk (top of lightbox)
- **Close button**: Click ✕ button, click outside, or press Escape to close
- **HiDPI scaling**: Screenshots display at correct size on Retina displays

Screenshots are base64-encoded PNGs returned in the tool result. The
`device_scale_factor` config (default: 2.0) controls the rendering resolution
for high-DPI displays.

### Telegram Integration

When using the Telegram channel, screenshots are automatically sent to the chat:

- Images sent as photos when dimensions are within Telegram limits
- Automatically retried as documents for oversized images (aspect ratio > 20)
- Error messages sent to channel if delivery fails

## Handling Model Errors

Some models (particularly Claude via GitHub Copilot) occasionally send malformed
tool calls with missing required fields. Moltis handles this gracefully:

- **Default action**: If `url` is provided but `action` is missing, defaults to
  `navigate`
- **Automatic retry**: The agent loop retries with corrected arguments
- **Muted error display**: Validation errors show as muted/informational cards
  in the UI (60% opacity, gray text) to indicate they're expected, not alarming

## Troubleshooting

### Browser not launching

- Ensure Chrome/Chromium is installed
- Check `chrome_path` in config if using custom location
- Set `CHROME` environment variable to specify browser path
- On Linux, install dependencies: `apt-get install chromium-browser`
- On macOS, if using Homebrew Chromium, prefer installing Google Chrome or Brave
  (the Homebrew chromium wrapper can be unreliable)

### Elements not found

- Use `snapshot` to see available elements
- Elements must be visible in the viewport
- Some elements may need scrolling first

### Timeouts

- Increase `navigation_timeout_ms` for slow pages
- Use `wait` action to wait for dynamic content
- Check network connectivity

### High memory usage

- Browser instances are now limited by memory (blocks at 90% by default)
- Set `max_instances > 0` for a hard limit if preferred
- Lower `idle_timeout_secs` to clean up faster
- Consider enabling headless mode if not already

### Pool exhaustion

- Browser tool now auto-tracks session IDs, preventing pool exhaustion from
  LLMs that forget to pass session_id
- If you still hit limits, check `memory_limit_percent` threshold
- Use `close` action when done to free up sessions
