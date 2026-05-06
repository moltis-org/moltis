use std::{borrow::Cow, fmt::Write, sync::Arc};

use {
    anyhow::{Result, bail},
    tracing::{debug, info, trace, warn},
};

#[cfg(feature = "metrics")]
use moltis_metrics::{counter, histogram, labels, llm as llm_metrics};

use moltis_common::hooks::{ChannelBinding, HookAction, HookPayload, HookRegistry};

use crate::{
    model::{
        ChatMessage, CompletionResponse, LlmProvider, StreamEvent, ToolCall, Usage, UserContent,
    },
    response_sanitizer::{clean_response, recover_tool_calls_from_content},
    tool_arg_validator::validate_tool_args,
    tool_loop_detector::{
        LoopDetectorAction, ToolCallFingerprint, ToolLoopDetector, format_intervention_message,
        format_strip_tools_message,
    },
    tool_parsing::{
        looks_like_failed_tool_call, new_synthetic_tool_call_id, parse_tool_calls_from_text,
    },
    tool_registry::ToolRegistry,
};

use futures::StreamExt;

mod core;
mod runner;
mod streaming;
mod tool_result;

use {core::*, runner::*, tool_result::*};
