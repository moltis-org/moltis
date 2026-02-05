# Streaming Architecture

This document explains how streaming responses work in Moltis, from the LLM
provider through to the web UI.

## Overview

Moltis supports real-time token streaming for LLM responses, providing a much
better user experience than waiting for the complete response. Streaming works
even when tools are enabled, allowing users to see text as it arrives while
tool calls are accumulated and executed.

## Components

### 1. StreamEvent Enum (`crates/agents/src/model.rs`)

The `StreamEvent` enum defines all events that can occur during a streaming
LLM response:

```rust
pub enum StreamEvent {
    /// Text content delta - a chunk of text from the LLM.
    Delta(String),

    /// A tool call has started (for providers with native tool support).
    ToolCallStart { id: String, name: String, index: usize },

    /// Streaming delta for tool call arguments (JSON fragment).
    ToolCallArgumentsDelta { index: usize, delta: String },

    /// A tool call's arguments are complete.
    ToolCallComplete { index: usize },

    /// Stream completed successfully with token usage.
    Done(Usage),

    /// An error occurred.
    Error(String),
}
```

### 2. LlmProvider Trait (`crates/agents/src/model.rs`)

The `LlmProvider` trait defines two streaming methods:

- `stream()` - Basic streaming without tool support
- `stream_with_tools()` - Streaming with tool schemas passed to the API

Providers that support streaming with tools (like Anthropic) override
`stream_with_tools()`. Others fall back to `stream()` which ignores the tools
parameter.

### 3. Anthropic Provider (`crates/agents/src/providers/anthropic.rs`)

The Anthropic provider implements streaming by:

1. Making a POST request to `/v1/messages` with `"stream": true`
2. Reading Server-Sent Events (SSE) from the response
3. Parsing events and yielding appropriate `StreamEvent` variants:

| SSE Event Type | StreamEvent |
|----------------|-------------|
| `content_block_start` (text) | (none, just tracking) |
| `content_block_start` (tool_use) | `ToolCallStart` |
| `content_block_delta` (text_delta) | `Delta` |
| `content_block_delta` (input_json_delta) | `ToolCallArgumentsDelta` |
| `content_block_stop` | `ToolCallComplete` (for tool blocks) |
| `message_delta` | (usage tracking) |
| `message_stop` | `Done` |
| `error` | `Error` |

### 4. Agent Runner (`crates/agents/src/runner.rs`)

The `run_agent_loop_streaming()` function orchestrates the streaming agent
loop:

```
┌─────────────────────────────────────────────────────────┐
│                    Agent Loop                           │
│                                                         │
│  1. Call provider.stream_with_tools()                   │
│                                                         │
│  2. While stream has events:                            │
│     ├─ Delta(text) → emit RunnerEvent::TextDelta        │
│     ├─ ToolCallStart → accumulate tool call             │
│     ├─ ToolCallArgumentsDelta → accumulate args         │
│     ├─ ToolCallComplete → finalize args                 │
│     ├─ Done → record usage                              │
│     └─ Error → return error                             │
│                                                         │
│  3. If no tool calls → return accumulated text          │
│                                                         │
│  4. Execute tool calls concurrently                     │
│     ├─ Emit ToolCallStart events                        │
│     ├─ Run tools in parallel                            │
│     └─ Emit ToolCallEnd events                          │
│                                                         │
│  5. Append tool results to messages                     │
│                                                         │
│  6. Loop back to step 1                                 │
└─────────────────────────────────────────────────────────┘
```

### 5. Gateway (`crates/gateway/src/chat.rs`)

The gateway's `run_with_tools()` function:

1. Sets up an event callback that broadcasts `RunnerEvent`s via WebSocket
2. Calls `run_agent_loop_streaming()`
3. Broadcasts events to connected clients as JSON frames

Event types broadcast to the UI:

| RunnerEvent | WebSocket State |
|-------------|-----------------|
| `Thinking` | `thinking` |
| `ThinkingDone` | `thinking_done` |
| `TextDelta(text)` | `delta` with `text` field |
| `ToolCallStart` | `tool_call_start` |
| `ToolCallEnd` | `tool_call_end` |
| `Iteration(n)` | `iteration` |

### 6. Frontend (`crates/gateway/src/assets/js/`)

The JavaScript frontend handles streaming via WebSocket:

1. **websocket.js** - Receives WebSocket frames and dispatches to handlers
2. **events.js** - Event bus for distributing events to components
3. **state.js** - Manages streaming state (`streamText`, `streamEl`)

When a `delta` event arrives:

```javascript
function handleChatDelta(p, isActive, isChatPage) {
  if (!(p.text && isActive && isChatPage)) return;
  removeThinking();
  if (!S.streamEl) {
    S.setStreamText("");
    S.setStreamEl(document.createElement("div"));
    S.streamEl.className = "msg assistant";
    S.chatMsgBox.appendChild(S.streamEl);
  }
  S.setStreamText(S.streamText + p.text);
  setSafeMarkdownHtml(S.streamEl, S.streamText);
  S.chatMsgBox.scrollTop = S.chatMsgBox.scrollHeight;
}
```

## Data Flow

```
┌──────────────┐     SSE      ┌──────────────┐   StreamEvent   ┌──────────────┐
│   Anthropic  │─────────────▶│   Provider   │────────────────▶│    Runner    │
│     API      │              │              │                 │              │
└──────────────┘              └──────────────┘                 └──────┬───────┘
                                                                      │
                                                               RunnerEvent
                                                                      │
                                                                      ▼
┌──────────────┐   WebSocket  ┌──────────────┐    Callback     ┌──────────────┐
│   Browser    │◀─────────────│   Gateway    │◀────────────────│   Callback   │
│              │              │              │                 │   (on_event) │
└──────────────┘              └──────────────┘                 └──────────────┘
```

## Adding Streaming to New Providers

To add streaming support for a new LLM provider:

1. Implement the `stream()` method (basic streaming)
2. If the provider supports tools in streaming mode, override
   `stream_with_tools()`
3. Parse the provider's streaming format and yield appropriate `StreamEvent`
   variants
4. Handle errors gracefully with `StreamEvent::Error`
5. Always emit `StreamEvent::Done` with usage statistics when complete

Example skeleton:

```rust
fn stream_with_tools(
    &self,
    messages: Vec<serde_json::Value>,
    tools: Vec<serde_json::Value>,
) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
    Box::pin(async_stream::stream! {
        // Make streaming request to provider API
        let resp = self.client.post(...)
            .json(&body)
            .send()
            .await?;

        // Read SSE or streaming response
        let mut byte_stream = resp.bytes_stream();

        while let Some(chunk) = byte_stream.next().await {
            // Parse chunk and yield events
            match parse_event(&chunk) {
                TextDelta(text) => yield StreamEvent::Delta(text),
                ToolStart { id, name, idx } => {
                    yield StreamEvent::ToolCallStart { id, name, index: idx }
                }
                // ... handle other event types
            }
        }

        yield StreamEvent::Done(usage);
    })
}
```

## Performance Considerations

- **Unbounded channels**: WebSocket send channels are unbounded, so slow
  clients can accumulate messages in memory
- **Markdown re-rendering**: The frontend re-renders full markdown on each
  delta, which is O(n) work per delta. For very long responses, this can
  cause UI lag
- **Concurrent tool execution**: Multiple tool calls are executed in parallel
  using `futures::join_all()`, improving throughput when the LLM requests
  several tools at once
