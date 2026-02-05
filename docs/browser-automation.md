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

Browser automation is **disabled by default** and requires explicit opt-in.
Add to your `moltis.toml`:

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
```

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

## Security Considerations

1. **Sandboxing**: Browser runs with `--no-sandbox` for container compatibility.
   For production, consider running in a sandboxed container.

2. **Resource limits**: Configure `max_instances` to prevent resource exhaustion.

3. **Idle cleanup**: Browsers are automatically closed after `idle_timeout_secs`
   of inactivity.

4. **Network access**: The browser has full network access. Use firewall rules
   if you need to restrict outbound connections.

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
