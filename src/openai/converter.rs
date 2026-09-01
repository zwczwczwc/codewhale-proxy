use crate::anthropic::types::{
    ContentBlockDeltaData, ContentBlockStartData, MessageDeltaData, MessageStartData,
    MessagesResponse, ResponseContentBlock, SseEvent, StreamUsage, Usage,
};
use crate::cache::{chat_usage_view, chat_usage_view_from_buckets, from_chat_usage, CachePolicy};
use crate::openai::types::{ChatCompletionResponse, ChatDelta};
use serde_json::Value;

/// Convert OpenAI non-streaming response to Anthropic format.
pub fn convert_non_stream_response(
    openai_resp: &ChatCompletionResponse,
    model: &str,
    msg_id: &str,
    reasoning_field: &str,
    reasoning_field_alt: &[String],
    cache_policy: Option<&CachePolicy>,
) -> MessagesResponse {
    let choice = openai_resp.choices.first();
    let message = choice.and_then(|c| c.message.as_ref());

    let mut content: Vec<ResponseContentBlock> = Vec::new();

    if let Some(msg) = message {
        // 1. Thinking block — use ProviderConfig.reasoning_field to select the field
        let reasoning = msg.get_reasoning(reasoning_field, reasoning_field_alt);
        if let Some(ref rc) = reasoning {
            if !rc.trim().is_empty() {
                content.push(ResponseContentBlock::Thinking {
                    thinking: rc.to_string(),
                    signature: "sig_proxy_placeholder".to_string(),
                });
            }
        }

        // 2. Text block
        if let Some(ref text) = msg.content {
            if !text.trim().is_empty() {
                content.push(ResponseContentBlock::Text { text: text.clone() });
            }
        }

        // 3. Tool use blocks
        if let Some(tool_calls) = &msg.tool_calls {
            for tc in tool_calls {
                let input: Value = tc
                    .function
                    .as_ref()
                    .and_then(|f| serde_json::from_str(&f.arguments).ok())
                    .unwrap_or(Value::Null);
                content.push(ResponseContentBlock::ToolUse {
                    id: tc.id.clone(),
                    name: tc
                        .function
                        .as_ref()
                        .map_or("unknown".to_string(), |f| f.name.clone()),
                    input,
                });
            }
        }
    }

    let stop_reason = choice
        .and_then(|c| c.finish_reason.as_ref())
        .map(|fr| map_finish_reason(fr));

    // Single policy-gated view: Legacy (default — policy None/off) reproduces
    // the historical wire byte-for-byte (read = ptd.cached_tokens only,
    // creation = the `prompt - cached` remainder label); Raw (explicit usage
    // source) reads top-level → nested → DeepSeek hit and never fabricates a
    // creation. Absent usage ⇒ an empty view ⇒ all-zero wire (unchanged).
    let view = chat_usage_view(openai_resp.usage.as_ref(), cache_policy);
    let usage = Usage {
        input_tokens: view.input.unwrap_or(0),
        output_tokens: openai_resp
            .usage
            .as_ref()
            .and_then(|u| u.completion_tokens)
            .unwrap_or(0),
        cache_read_input_tokens: view.read,
        cache_creation_input_tokens: view.creation,
    };

    MessagesResponse {
        id: msg_id.to_string(),
        response_type: "message".to_string(),
        role: "assistant".to_string(),
        content,
        model: model.to_string(),
        stop_reason,
        stop_sequence: None,
        usage,
    }
}

fn map_finish_reason(fr: &str) -> String {
    match fr {
        "stop" => "end_turn".to_string(),
        "tool_calls" => "tool_use".to_string(),
        "length" => "max_tokens".to_string(),
        other => other.to_string(),
    }
}

/// Empty-text response guard mode (CC_PROXY_EMPTY_TEXT_GUARD).
///
/// When upstream reaches a terminal state with 0 text blocks and 0 tool_use
/// blocks, the converted response would otherwise be a silent empty
/// `message_stop` — which clients (e.g. Claude Code compaction) treat as a
/// success and then fail downstream with "summarization produced empty
/// response". This guard makes the anomaly observable:
///
/// - `Off` — no detection, legacy behavior (silent empty message_stop).
/// - `Warn` — detect + structured log, but forward the empty response unchanged
///   (observation period, default).
/// - `Enforce` — detect and emit a stream `error` event instead of the empty
///   message_delta/message_stop (fail-closed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmptyTextGuard {
    Off,
    Warn,
    Enforce,
}

impl EmptyTextGuard {
    pub fn from_env() -> Self {
        match std::env::var("CC_PROXY_EMPTY_TEXT_GUARD").as_deref() {
            Ok("off") => EmptyTextGuard::Off,
            Ok("enforce") => EmptyTextGuard::Enforce,
            // Default: warn — observation period before fail-closed.
            _ => EmptyTextGuard::Warn,
        }
    }
}

/// SSE stream state machine — tracks block transitions during streaming.
pub struct SseStateMachine {
    content_index: u32,
    text_started: bool,
    thinking_started: bool,
    tool_indices: std::collections::HashMap<u32, u32>,
    is_reasoning_model: bool,
    /// True once a non-whitespace text delta has been emitted in this stream.
    /// Used by the empty-text guard to distinguish "upstream completed with no
    /// text at all" from a normal response. Reset per stream (new instance).
    emitted_text: bool,
    /// Empty-text guard mode for this stream (read once from env at
    /// process_stream top; tests may set explicitly).
    empty_text_guard: EmptyTextGuard,
    /// Primary field name for reasoning content (from ProviderConfig.reasoning_field)
    reasoning_field: String,
    /// Alternative field names to try if primary is empty/missing
    reasoning_field_alt: Vec<String>,
    /// Accumulated text for current text block
    current_text: String,
    /// Accumulated thinking for current thinking block
    current_thinking: String,
    /// Track tool call names for content_block_start
    tool_names: std::collections::HashMap<u32, String>,
    /// Track if signature_delta has been sent for current thinking block
    thinking_signature_sent: bool,
    /// Track if message_start has been sent
    message_start_sent: bool,
    /// Input tokens from usage
    input_tokens: Option<u32>,
    /// Cached tokens from prompt_tokens_details (standard OpenAI format)
    cached_tokens: Option<u32>,
    /// Declarative cache policy for this request (None/off ⇒ legacy wire/log).
    cache_policy: Option<CachePolicy>,
    /// Canonical raw read (top-level `cached_tokens` → nested ptd → DeepSeek
    /// hit) captured from usage-only chunks; used only under an opt-in policy.
    raw_read_tokens: Option<u32>,
}

impl SseStateMachine {
    pub fn new(
        is_reasoning_model: bool,
        reasoning_field: String,
        reasoning_field_alt: Vec<String>,
        cache_policy: Option<CachePolicy>,
    ) -> Self {
        Self {
            content_index: 0,
            text_started: false,
            thinking_started: false,
            tool_indices: std::collections::HashMap::new(),
            is_reasoning_model,
            emitted_text: false,
            empty_text_guard: EmptyTextGuard::Warn,
            reasoning_field,
            reasoning_field_alt,
            current_text: String::new(),
            current_thinking: String::new(),
            tool_names: std::collections::HashMap::new(),
            thinking_signature_sent: false,
            message_start_sent: false,
            input_tokens: None,
            cached_tokens: None,
            cache_policy,
            raw_read_tokens: None,
        }
    }

    /// Set the empty-text guard mode for this stream (read from env by the
    /// process_stream caller; explicit setter keeps `new()` signature stable
    /// for the many test construction sites).
    pub fn set_empty_text_guard(&mut self, guard: EmptyTextGuard) {
        self.empty_text_guard = guard;
    }

    /// Current empty-text guard mode (test helper).
    #[cfg(test)]
    pub fn empty_text_guard(&self) -> EmptyTextGuard {
        self.empty_text_guard
    }

    /// Process a single SSE delta chunk. Returns Vec of SSE events to emit.
    pub fn process_delta(
        &mut self,
        delta: &ChatDelta,
        usage: Option<&crate::openai::types::Usage>,
    ) -> Vec<SseEvent> {
        let mut events = Vec::new();

        // Handle usage-only chunks (no choices/delta)
        let has_reasoning = delta
            .get_reasoning(&self.reasoning_field, &self.reasoning_field_alt)
            .is_some();
        if !has_reasoning
            && delta.content.is_empty()
            && delta.tool_calls.is_none()
            && delta.role.is_none()
        {
            if let Some(usage) = usage {
                self.input_tokens = usage.prompt_tokens;
                self.cached_tokens = usage
                    .prompt_tokens_details
                    .as_ref()
                    .and_then(|d| d.cached_tokens);
                self.raw_read_tokens = from_chat_usage(usage).cache_read_tokens;
            }
            return events;
        }

        // 1. Process reasoning — use ProviderConfig.reasoning_field to select the field
        let reasoning_delta = delta.get_reasoning(&self.reasoning_field, &self.reasoning_field_alt);
        if let Some(ref rc) = reasoning_delta {
            if !rc.is_empty() && self.is_reasoning_model {
                if !self.thinking_started {
                    // Close any open text block first
                    if self.text_started {
                        events.push(SseEvent::ContentBlockStop {
                            index: self.content_index - 1,
                        });
                        self.text_started = false;
                    }
                    // Start thinking block
                    events.push(SseEvent::ContentBlockStart {
                        index: self.content_index,
                        content_block: ContentBlockStartData::Thinking {
                            thinking: String::new(),
                            signature: String::new(),
                        },
                    });
                    self.thinking_started = true;
                    self.thinking_signature_sent = false;
                    self.current_thinking = String::new();
                }
                self.current_thinking.push_str(rc);
                events.push(SseEvent::ContentBlockDelta {
                    index: self.content_index,
                    delta: ContentBlockDeltaData::ThinkingDelta {
                        thinking: rc.to_string(),
                    },
                });
            }
        }

        // 2. Process content
        if let Some(ref text) = delta.content.text {
            if !text.is_empty() {
                // Close thinking block if open
                if self.thinking_started {
                    // Send signature_delta before closing thinking block
                    if !self.thinking_signature_sent {
                        events.push(SseEvent::ContentBlockDelta {
                            index: self.content_index,
                            delta: ContentBlockDeltaData::SignatureDelta {
                                signature: "sig_proxy_placeholder".to_string(),
                            },
                        });
                        self.thinking_signature_sent = true;
                    }
                    events.push(SseEvent::ContentBlockStop {
                        index: self.content_index,
                    });
                    self.content_index += 1;
                    self.thinking_started = false;
                }
                if !self.text_started {
                    events.push(SseEvent::ContentBlockStart {
                        index: self.content_index,
                        content_block: ContentBlockStartData::Text {
                            text: String::new(),
                        },
                    });
                    self.text_started = true;
                    self.current_text = String::new();
                }
                self.current_text.push_str(text);
                // Mark that real (non-whitespace) text has been emitted — this
                // stream is NOT an empty-text response.
                if !text.trim().is_empty() {
                    self.emitted_text = true;
                }
                events.push(SseEvent::ContentBlockDelta {
                    index: self.content_index,
                    delta: ContentBlockDeltaData::TextDelta { text: text.clone() },
                });
            }
        }

        // 3. Process tool_calls
        if let Some(tool_calls) = &delta.tool_calls {
            for tc in tool_calls {
                let tc_index = tc.index.unwrap_or(0);
                let sse_index = if let Some(&idx) = self.tool_indices.get(&tc_index) {
                    idx
                } else {
                    // New tool call
                    let new_idx = if self.thinking_started || self.text_started {
                        // Close current block
                        if self.thinking_started {
                            if !self.thinking_signature_sent {
                                events.push(SseEvent::ContentBlockDelta {
                                    index: self.content_index,
                                    delta: ContentBlockDeltaData::SignatureDelta {
                                        signature: "sig_proxy_placeholder".to_string(),
                                    },
                                });
                                self.thinking_signature_sent = true;
                            }
                            events.push(SseEvent::ContentBlockStop {
                                index: self.content_index,
                            });
                            self.content_index += 1;
                            self.thinking_started = false;
                        }
                        if self.text_started {
                            events.push(SseEvent::ContentBlockStop {
                                index: self.content_index,
                            });
                            self.content_index += 1;
                            self.text_started = false;
                        }
                        self.content_index
                    } else {
                        self.content_index
                    };
                    self.tool_indices.insert(tc_index, new_idx);

                    // Get tool call id and name correctly (P1-1: fix id/name swap)
                    // id: from tc.id (call ID), name: from tc.function.name (tool name)
                    let call_id = tc.id.clone().unwrap_or_else(|| "unknown_call".to_string());
                    let tool_name = if let Some(ref func) = tc.function {
                        func.name
                            .clone()
                            .unwrap_or_else(|| "unknown_tool".to_string())
                    } else {
                        "unknown_tool".to_string()
                    };

                    self.tool_names.insert(new_idx, tool_name.clone());

                    events.push(SseEvent::ContentBlockStart {
                        index: new_idx,
                        content_block: ContentBlockStartData::ToolUse {
                            id: call_id,
                            name: tool_name,
                            input: Value::Object(serde_json::Map::new()),
                        },
                    });
                    self.content_index += 1;

                    new_idx
                };

                // Handle tool call delta
                if let Some(ref func) = tc.function {
                    if let Some(ref name) = func.name {
                        // Update name in tool_names
                        self.tool_names.insert(sse_index, name.clone());
                    }
                    if let Some(ref args) = func.arguments {
                        events.push(SseEvent::ContentBlockDelta {
                            index: sse_index,
                            delta: ContentBlockDeltaData::InputJsonDelta {
                                partial_json: args.clone(),
                            },
                        });
                    }
                }
                // Handle tool call id
                if let Some(ref id) = tc.id {
                    self.tool_names.insert(sse_index, id.clone());
                }
            }
        }

        events
    }

    /// Close all open blocks and return final events (message_delta + message_stop).
    pub fn finalize(
        &mut self,
        stop_reason: Option<&str>,
        output_tokens: Option<u32>,
        usage: Option<&crate::openai::types::Usage>,
    ) -> Vec<SseEvent> {
        // Empty-text guard: if upstream reached a terminal state but emitted no
        // text and no tool_use blocks, the converted Anthropic response would be
        // a silent empty message_stop — which clients (Claude Code compaction)
        // treat as success and then fail downstream. Make it observable.
        if !self.emitted_text && self.tool_indices.is_empty() {
            match self.empty_text_guard {
                EmptyTextGuard::Off => {}
                EmptyTextGuard::Warn => {
                    tracing::warn!(
                        stop_reason = ?stop_reason,
                        output_tokens = ?output_tokens,
                        thinking_chars = self.current_thinking.len(),
                        "upstream completed with 0 text blocks and 0 tool_use blocks \
                         (empty response); WARN mode — forwarding unchanged"
                    );
                }
                EmptyTextGuard::Enforce => {
                    let reason = stop_reason.unwrap_or("(none)");
                    tracing::warn!(
                        stop_reason = ?stop_reason,
                        output_tokens = ?output_tokens,
                        thinking_chars = self.current_thinking.len(),
                        "upstream completed with 0 text blocks and 0 tool_use blocks \
                         (empty response); ENFORCE mode — emitting stream error"
                    );
                    return vec![SseEvent::Error {
                        error: crate::anthropic::types::ErrorData {
                            error_type: "stream_error".to_string(),
                            message: format!(
                                "upstream returned empty content (finish_reason={reason}, \
                                 output_tokens={}, thinking_chars={})",
                                output_tokens.unwrap_or(0),
                                self.current_thinking.len(),
                            ),
                        },
                    }];
                }
            }
        }

        let mut events = Vec::new();

        // Close thinking block
        if self.thinking_started {
            if !self.thinking_signature_sent {
                events.push(SseEvent::ContentBlockDelta {
                    index: self.content_index,
                    delta: ContentBlockDeltaData::SignatureDelta {
                        signature: "sig_proxy_placeholder".to_string(),
                    },
                });
            }
            events.push(SseEvent::ContentBlockStop {
                index: self.content_index,
            });
            self.thinking_started = false;
        }

        // Close text block
        if self.text_started {
            events.push(SseEvent::ContentBlockStop {
                index: self.content_index,
            });
            self.text_started = false;
        }

        // Close all tool blocks
        for (&_tc_idx, &sse_idx) in &self.tool_indices {
            events.push(SseEvent::ContentBlockStop { index: sse_idx });
        }

        // Message delta — prefer usage from the chunk, fall back to self state.
        // A single policy-gated view computes the wire buckets: Legacy (default)
        // reproduces the historical read/creation byte-for-byte; Raw reads the
        // canonical read (top-level → nested → DeepSeek hit) and never
        // fabricates a creation.
        let mapped_reason = stop_reason.map(map_finish_reason);
        let input_tokens = usage.and_then(|u| u.prompt_tokens).or(self.input_tokens);
        let legacy_read = usage
            .and_then(|u| u.prompt_tokens_details.as_ref())
            .and_then(|d| d.cached_tokens)
            .or(self.cached_tokens);
        let raw_read = usage
            .and_then(|u| from_chat_usage(u).cache_read_tokens)
            .or(self.raw_read_tokens);
        let view = chat_usage_view_from_buckets(
            input_tokens,
            legacy_read,
            raw_read,
            self.cache_policy.as_ref(),
        );
        events.push(SseEvent::MessageDelta {
            delta: MessageDeltaData {
                stop_reason: mapped_reason,
                stop_sequence: None,
            },
            usage: Some(StreamUsage {
                input_tokens: view.input,
                output_tokens,
                cache_read_input_tokens: view.read,
                cache_creation_input_tokens: view.creation,
            }),
        });

        // Message stop
        events.push(SseEvent::MessageStop);

        events
    }

    /// Generate the message_start event (should be sent first in the stream).
    pub fn message_start(&mut self, model: &str, msg_id: &str) -> SseEvent {
        self.message_start_sent = true;
        SseEvent::MessageStart {
            message: MessageStartData {
                id: msg_id.to_string(),
                msg_type: "message".to_string(),
                role: "assistant".to_string(),
                content: vec![],
                model: model.to_string(),
                // Usage must be an object with non-null numeric fields, not
                // null: Claude Code's Agent tool dereferences
                // message_start.message.usage.input_tokens and crashes on null
                // ("null is not an object (evaluating 'o.input_tokens')").
                // Zero placeholders are allowed here — real input/output/cache
                // usage is only known from the upstream terminal chunk and is
                // reported on the terminal message_delta; these zeros are never
                // a fabricated cache read/write.
                usage: Some(StreamUsage {
                    input_tokens: Some(0),
                    output_tokens: Some(0),
                    cache_read_input_tokens: Some(0),
                    cache_creation_input_tokens: Some(0),
                }),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_non_stream_with_thinking() {
        let openai_resp = ChatCompletionResponse {
            id: "chatcmpl-123".to_string(),
            choices: vec![crate::openai::types::Choice {
                index: 0,
                message: Some(crate::openai::types::ChatMessage {
                    role: Some("assistant".to_string()),
                    content: Some("answer".to_string()),
                    reasoning_content: Some("let me think".to_string()),
                    reasoning: None,
                    tool_calls: None,
                }),
                delta: None,
                finish_reason: Some("stop".to_string()),
            }],
            usage: Some(crate::openai::types::Usage {
                prompt_tokens: Some(10),
                completion_tokens: Some(5),
                total_tokens: Some(15),
                prompt_cache_hit_tokens: None,
                prompt_cache_miss_tokens: None,
                cached_tokens: None,
                prompt_tokens_details: None,
            }),
        };

        let result = convert_non_stream_response(
            &openai_resp,
            "deepseek-v4",
            "msg_123",
            "reasoning_content",
            &[],
            None,
        );
        assert_eq!(result.content.len(), 2);
        assert!(matches!(
            result.content[0],
            ResponseContentBlock::Thinking { .. }
        ));
        assert!(matches!(
            result.content[1],
            ResponseContentBlock::Text { .. }
        ));
        assert_eq!(result.stop_reason, Some("end_turn".to_string()));
    }

    // --- Phase 2b.4: Chat usage view on the non-stream wire ---

    fn raw_policy() -> CachePolicy {
        CachePolicy {
            usage: crate::cache::UsagePolicy::TopLevelCachedTokens,
            prompt_cache_key_enabled: false,
            upstream: None,
            effort_enum: None,
            replay: crate::cache::ReplayPolicy::Off,
            history: crate::cache::HistoryPolicy::Off,
            relocate: crate::cache::RelocatePolicy::Off,
            pinned_effort: None,
        }
    }

    fn response_with_usage(usage: serde_json::Value) -> ChatCompletionResponse {
        ChatCompletionResponse {
            id: "chatcmpl-1".to_string(),
            choices: vec![crate::openai::types::Choice {
                index: 0,
                message: Some(crate::openai::types::ChatMessage {
                    role: Some("assistant".to_string()),
                    content: Some("answer".to_string()),
                    reasoning_content: None,
                    reasoning: None,
                    tool_calls: None,
                }),
                delta: None,
                finish_reason: Some("stop".to_string()),
            }],
            usage: Some(serde_json::from_value(usage).unwrap()),
        }
    }

    fn convert_usage(usage: serde_json::Value, policy: Option<&CachePolicy>) -> Usage {
        convert_non_stream_response(
            &response_with_usage(usage),
            "kimi-k3-turbo",
            "msg_1",
            "reasoning_content",
            &[],
            policy,
        )
        .usage
    }

    #[test]
    fn chat_non_stream_usage_legacy_matches_historical_wire_exactly() {
        // Default (None) policy: read = ptd.cached_tokens only, creation = the
        // historical `prompt - cached` remainder label.
        let usage = convert_usage(
            serde_json::json!({
                "prompt_tokens": 100, "completion_tokens": 20, "total_tokens": 120,
                "prompt_tokens_details": {"cached_tokens": 70},
            }),
            None,
        );
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 20);
        assert_eq!(usage.cache_read_input_tokens, Some(70));
        assert_eq!(usage.cache_creation_input_tokens, Some(30));
    }

    #[test]
    fn chat_non_stream_usage_legacy_ignores_top_level_cached_tokens() {
        // Kimi top-level cached_tokens is invisible to non-opt-in providers:
        // the historical wire stays ptd-only (read None, creation None).
        let usage = convert_usage(
            serde_json::json!({
                "prompt_tokens": 100, "completion_tokens": 20, "total_tokens": 120,
                "cached_tokens": 70,
            }),
            None,
        );
        assert_eq!(usage.cache_read_input_tokens, None);
        assert_eq!(usage.cache_creation_input_tokens, None);
    }

    #[test]
    fn chat_non_stream_usage_raw_opt_in_reads_top_level_and_never_fabricates_creation() {
        let usage = convert_usage(
            serde_json::json!({
                "prompt_tokens": 100, "completion_tokens": 20, "total_tokens": 120,
                "cached_tokens": 70,
            }),
            Some(&raw_policy()),
        );
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 20);
        assert_eq!(usage.cache_read_input_tokens, Some(70));
        assert_eq!(usage.cache_creation_input_tokens, None);
    }

    #[test]
    fn chat_non_stream_usage_raw_falls_back_to_nested_when_top_level_absent() {
        let usage = convert_usage(
            serde_json::json!({
                "prompt_tokens": 100, "completion_tokens": 20, "total_tokens": 120,
                "prompt_tokens_details": {"cached_tokens": 60},
            }),
            Some(&raw_policy()),
        );
        assert_eq!(usage.cache_read_input_tokens, Some(60));
        assert_eq!(usage.cache_creation_input_tokens, None);
    }

    #[test]
    fn chat_non_stream_usage_raw_deepseek_hit_is_read_and_creation_stays_none() {
        let usage = convert_usage(
            serde_json::json!({
                "prompt_tokens": 100, "completion_tokens": 20, "total_tokens": 120,
                "prompt_cache_hit_tokens": 60, "prompt_cache_miss_tokens": 40,
            }),
            Some(&raw_policy()),
        );
        assert_eq!(usage.cache_read_input_tokens, Some(60));
        assert_eq!(usage.cache_creation_input_tokens, None);
    }

    #[test]
    fn chat_non_stream_usage_without_usage_emits_zero_wire() {
        // Absent usage object: all-zero wire (unchanged legacy behavior) — an
        // HTTP error / missing usage is never a cache miss on the wire.
        let resp = ChatCompletionResponse {
            id: "chatcmpl-1".to_string(),
            choices: vec![],
            usage: None,
        };
        let converted = convert_non_stream_response(
            &resp,
            "kimi-k3-turbo",
            "msg_1",
            "reasoning_content",
            &[],
            None,
        );
        assert_eq!(converted.usage.input_tokens, 0);
        assert_eq!(converted.usage.output_tokens, 0);
        assert_eq!(converted.usage.cache_read_input_tokens, None);
        assert_eq!(converted.usage.cache_creation_input_tokens, None);
    }

    // --- Phase 2b.4: Chat usage view on the stream final message_delta ---

    fn usage_only_delta() -> ChatDelta {
        ChatDelta {
            role: None,
            content: Default::default(),
            reasoning_content: Default::default(),
            reasoning: Default::default(),
            tool_calls: None,
        }
    }

    fn message_delta_usage(events: &[SseEvent]) -> StreamUsage {
        events
            .iter()
            .find_map(|event| match event {
                SseEvent::MessageDelta { usage: Some(u), .. } => Some(u.clone()),
                _ => None,
            })
            .expect("message_delta with usage present")
    }

    fn usage(value: serde_json::Value) -> crate::openai::types::Usage {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn chat_stream_final_usage_legacy_preserves_historical_wire() {
        // Usage-only chunk (nested ptd) → finalize → legacy message_delta.usage
        // keeps read = ptd.cached_tokens and creation = prompt - cached.
        let mut sm = SseStateMachine::new(false, "reasoning_content".into(), vec![], None);
        let u = usage(serde_json::json!({
            "prompt_tokens": 100, "completion_tokens": 20, "total_tokens": 120,
            "prompt_tokens_details": {"cached_tokens": 70},
        }));
        let events = sm.process_delta(&usage_only_delta(), Some(&u));
        assert!(events.is_empty(), "usage-only chunk emits no events");
        let final_events = sm.finalize(Some("stop"), Some(20), Some(&u));
        let md = message_delta_usage(&final_events);
        assert_eq!(md.input_tokens, Some(100));
        assert_eq!(md.output_tokens, Some(20));
        assert_eq!(md.cache_read_input_tokens, Some(70));
        assert_eq!(
            md.cache_creation_input_tokens,
            Some(30),
            "historical remainder label"
        );
        // Terminal sequence unchanged: message_delta then message_stop.
        assert!(matches!(final_events.last(), Some(SseEvent::MessageStop)));
    }

    #[test]
    fn chat_stream_final_usage_raw_opt_in_reads_top_level_cached_tokens() {
        let mut sm = SseStateMachine::new(
            false,
            "reasoning_content".into(),
            vec![],
            Some(raw_policy()),
        );
        let u = usage(serde_json::json!({
            "prompt_tokens": 100, "completion_tokens": 20, "total_tokens": 120,
            "cached_tokens": 70,
        }));
        let _ = sm.process_delta(&usage_only_delta(), Some(&u));
        let final_events = sm.finalize(Some("stop"), Some(20), Some(&u));
        let md = message_delta_usage(&final_events);
        assert_eq!(md.input_tokens, Some(100));
        assert_eq!(md.cache_read_input_tokens, Some(70));
        assert_eq!(
            md.cache_creation_input_tokens, None,
            "creation never fabricated"
        );
    }

    #[test]
    fn chat_stream_final_usage_raw_top_level_wins_over_nested_no_double_count() {
        let mut sm = SseStateMachine::new(
            false,
            "reasoning_content".into(),
            vec![],
            Some(raw_policy()),
        );
        let u = usage(serde_json::json!({
            "prompt_tokens": 100, "completion_tokens": 20, "total_tokens": 120,
            "cached_tokens": 70, "prompt_tokens_details": {"cached_tokens": 60},
        }));
        let final_events = sm.finalize(Some("stop"), Some(20), Some(&u));
        let md = message_delta_usage(&final_events);
        assert_eq!(md.cache_read_input_tokens, Some(70));
        assert_eq!(md.cache_creation_input_tokens, None);
    }

    #[test]
    fn chat_stream_final_usage_raw_falls_back_to_nested_when_top_level_absent() {
        let mut sm = SseStateMachine::new(
            false,
            "reasoning_content".into(),
            vec![],
            Some(raw_policy()),
        );
        let u = usage(serde_json::json!({
            "prompt_tokens": 100, "completion_tokens": 20, "total_tokens": 120,
            "prompt_tokens_details": {"cached_tokens": 60},
        }));
        let _ = sm.process_delta(&usage_only_delta(), Some(&u));
        let final_events = sm.finalize(Some("stop"), Some(20), Some(&u));
        let md = message_delta_usage(&final_events);
        assert_eq!(md.cache_read_input_tokens, Some(60));
        assert_eq!(md.cache_creation_input_tokens, None);
    }

    #[test]
    fn chat_stream_final_usage_legacy_top_level_only_reads_unknown() {
        // Legacy only reads ptd: top-level-only usage ⇒ read None and creation
        // None on the wire (unchanged), even though opt-in would read top-level.
        let mut sm = SseStateMachine::new(false, "reasoning_content".into(), vec![], None);
        let u = usage(serde_json::json!({
            "prompt_tokens": 100, "completion_tokens": 20, "total_tokens": 120,
            "cached_tokens": 70,
        }));
        let final_events = sm.finalize(Some("stop"), Some(20), Some(&u));
        let md = message_delta_usage(&final_events);
        assert_eq!(md.cache_read_input_tokens, None);
        assert_eq!(md.cache_creation_input_tokens, None);
    }

    #[test]
    fn chat_stream_final_usage_legacy_uses_captured_state_when_final_chunk_lacks_usage() {
        // eswitch can send the usage-only chunk then a terminal chunk without
        // usage; legacy wire falls back to the captured ptd state exactly.
        let mut sm = SseStateMachine::new(false, "reasoning_content".into(), vec![], None);
        let u = usage(serde_json::json!({
            "prompt_tokens": 100, "completion_tokens": 20, "total_tokens": 120,
            "prompt_tokens_details": {"cached_tokens": 70},
        }));
        let _ = sm.process_delta(&usage_only_delta(), Some(&u));
        let final_events = sm.finalize(Some("stop"), Some(20), None);
        let md = message_delta_usage(&final_events);
        assert_eq!(
            md.input_tokens,
            Some(100),
            "input falls back to captured state"
        );
        assert_eq!(md.cache_read_input_tokens, Some(70));
        assert_eq!(md.cache_creation_input_tokens, Some(30));
    }

    #[test]
    fn chat_stream_final_usage_raw_uses_captured_raw_read_when_final_chunk_lacks_usage() {
        let mut sm = SseStateMachine::new(
            false,
            "reasoning_content".into(),
            vec![],
            Some(raw_policy()),
        );
        let u = usage(serde_json::json!({
            "prompt_tokens": 100, "completion_tokens": 20, "total_tokens": 120,
            "cached_tokens": 70,
        }));
        let _ = sm.process_delta(&usage_only_delta(), Some(&u));
        let final_events = sm.finalize(Some("stop"), Some(20), None);
        let md = message_delta_usage(&final_events);
        assert_eq!(md.input_tokens, Some(100));
        assert_eq!(
            md.cache_read_input_tokens,
            Some(70),
            "raw read from captured usage-only chunk"
        );
        assert_eq!(md.cache_creation_input_tokens, None);
    }

    #[test]
    fn chat_stream_finalize_without_usage_keeps_none_not_zero() {
        let mut sm = SseStateMachine::new(false, "reasoning_content".into(), vec![], None);
        let final_events = sm.finalize(None, None, None);
        let md = message_delta_usage(&final_events);
        assert_eq!(md.input_tokens, None);
        assert_eq!(md.output_tokens, None);
        assert_eq!(md.cache_read_input_tokens, None);
        assert_eq!(md.cache_creation_input_tokens, None);
        assert!(matches!(final_events.last(), Some(SseEvent::MessageStop)));
    }

    // ============ STREAM DELTA SILENT-LOSS REGRESSION (GREEN) ============
    // kimi-k3-class upstreams send content/reasoning as arrays of parts. These
    // must decode into the same content-block frames as the string form.

    #[test]
    fn process_delta_array_content_emits_text_block() {
        let mut sm = SseStateMachine::new(false, "reasoning_content".into(), vec![], None);
        let delta: ChatDelta = serde_json::from_value(serde_json::json!({
            "content": [{"type": "text", "text": "Hello"}, {"type": "text", "text": " world"}]
        }))
        .unwrap();
        let events = sm.process_delta(&delta, None);
        assert!(
            matches!(events[0], SseEvent::ContentBlockStart { .. }),
            "expected ContentBlockStart first: {events:?}"
        );
        assert!(matches!(
            events[1],
            SseEvent::ContentBlockDelta {
                delta: ContentBlockDeltaData::TextDelta { .. },
                ..
            }
        ));
        if let SseEvent::ContentBlockDelta {
            delta: ContentBlockDeltaData::TextDelta { text },
            ..
        } = &events[1]
        {
            assert_eq!(
                text, "Hello world",
                "array parts concatenate into one text delta"
            );
        }
    }

    #[test]
    fn process_delta_reasoning_array_goes_to_thinking_not_text() {
        // A `reasoning` part array must produce a thinking block, never a text
        // block (thinking is not misread as ordinary text).
        let mut sm = SseStateMachine::new(true, "reasoning".into(), vec![], None);
        let delta: ChatDelta = serde_json::from_value(serde_json::json!({
            "reasoning": [{"type": "reasoning_summary_text", "summary_text": "let me think"}]
        }))
        .unwrap();
        let events = sm.process_delta(&delta, None);
        assert!(matches!(
            events[0],
            SseEvent::ContentBlockStart {
                content_block: ContentBlockStartData::Thinking { .. },
                ..
            }
        ));
        assert!(matches!(
            events[1],
            SseEvent::ContentBlockDelta {
                delta: ContentBlockDeltaData::ThinkingDelta { .. },
                ..
            }
        ));
    }

    #[test]
    fn process_delta_tool_calls_not_treated_as_text() {
        // Tool-call deltas still route to tool_use blocks, untouched by the
        // tolerant content decode.
        let mut sm = SseStateMachine::new(false, "reasoning_content".into(), vec![], None);
        let delta: ChatDelta = serde_json::from_value(serde_json::json!({
            "tool_calls": [{
                "index": 0, "id": "call_1", "type": "function",
                "function": {"name": "get_weather", "arguments": "{\"city\":\"SF\"}"}
            }]
        }))
        .unwrap();
        let events = sm.process_delta(&delta, None);
        assert!(
            matches!(
                events[0],
                SseEvent::ContentBlockStart {
                    content_block: ContentBlockStartData::ToolUse { .. },
                    ..
                }
            ),
            "tool delta must produce a tool_use block, not text: {events:?}"
        );
    }

    // ============ B10: Chat message_start usage must be a non-null object ============
    // Claude Code's Agent tool dereferences `message_start.message.usage.input_tokens`
    // and crashes when it is null ("null is not an object (evaluating
    // 'o.input_tokens')"). The first event of the Chat stream must carry a usage
    // OBJECT whose numeric fields are non-null numbers. Zero placeholders are
    // permitted here — real input/output/cache usage is only known from the
    // upstream terminal chunk and belongs on the terminal message_delta.

    #[test]
    fn chat_stream_message_start_usage_is_non_null_object() {
        let mut sm = SseStateMachine::new(false, "reasoning_content".into(), vec![], None);
        let event = sm.message_start("kimi-k3-turbo", "msg_b10");
        let value = serde_json::to_value(&event).expect("message_start serializes");
        let usage = &value["message"]["usage"];
        assert!(
            usage.is_object(),
            "message_start.message.usage must be an object, got: {value}"
        );
        for field in [
            "input_tokens",
            "output_tokens",
            "cache_read_input_tokens",
            "cache_creation_input_tokens",
        ] {
            assert!(
                usage.get(field).is_some_and(serde_json::Value::is_number),
                "message_start.usage.{field} must be a non-null number, got: {usage}"
            );
        }
    }

    #[test]
    fn chat_stream_message_start_placeholder_never_overrides_terminal_usage() {
        // B10 guard: the zero placeholders on message_start are never real
        // cache read/write. The terminal message_delta still carries the real
        // upstream buckets, and message_start never pretends to be them.
        let mut sm = SseStateMachine::new(false, "reasoning_content".into(), vec![], None);
        let start = sm.message_start("kimi-k3-turbo", "msg_b10");
        let u = usage(serde_json::json!({
            "prompt_tokens": 100, "completion_tokens": 20, "total_tokens": 120,
            "prompt_tokens_details": {"cached_tokens": 70},
        }));
        let _ = sm.process_delta(&usage_only_delta(), Some(&u));
        let final_events = sm.finalize(Some("stop"), Some(20), Some(&u));
        let md = message_delta_usage(&final_events);
        assert_eq!(md.input_tokens, Some(100));
        assert_eq!(md.output_tokens, Some(20));
        assert_eq!(md.cache_read_input_tokens, Some(70));
        assert_eq!(md.cache_creation_input_tokens, Some(30));
        // The message_start placeholder is not the terminal usage.
        let start_value = serde_json::to_value(&start).expect("message_start serializes");
        let su = &start_value["message"]["usage"];
        assert_ne!(
            su["input_tokens"], 100,
            "message_start must not carry terminal usage: {start_value}"
        );
    }

    // ============ Empty-text response guard (plan-B v1.1) ============
    // Upstream can reach a terminal state (finish_reason present) with 0 text
    // blocks and 0 tool_use blocks — e.g. glm-5.3 with effort=max burns the whole
    // output budget on reasoning. Previously this became a silent empty
    // message_stop; Claude Code compaction then failed with "summarization
    // produced empty response". Guard modes: Off (legacy), Warn (log only,
    // default), Enforce (emit stream error instead of empty message_stop).

    #[test]
    fn empty_text_guard_default_is_warn() {
        let sm = SseStateMachine::new(false, "reasoning_content".into(), vec![], None);
        assert_eq!(sm.empty_text_guard(), EmptyTextGuard::Warn);
    }

    #[test]
    fn empty_text_guard_enforce_empty_returns_error_not_message_stop() {
        let mut sm = SseStateMachine::new(false, "reasoning_content".into(), vec![], None);
        sm.set_empty_text_guard(EmptyTextGuard::Enforce);
        // No text, no tool_use, terminal with stop — the compaction failure shape.
        let events = sm.finalize(Some("stop"), Some(20), None);
        assert_eq!(
            events.len(),
            1,
            "enforce must emit only the error frame: {events:?}"
        );
        match &events[0] {
            SseEvent::Error { error } => {
                assert_eq!(error.error_type, "stream_error");
                assert!(
                    error.message.contains("upstream returned empty content"),
                    "message must be diagnostic: {}",
                    error.message
                );
                assert!(error.message.contains("finish_reason=stop"));
            }
            other => panic!("expected SseEvent::Error, got {other:?}"),
        }
    }

    #[test]
    fn empty_text_guard_enforce_length_with_thinking_returns_error() {
        // The suspected real failure shape: reasoning-only + finish_reason=length.
        let mut sm = SseStateMachine::new(true, "reasoning_content".into(), vec![], None);
        sm.set_empty_text_guard(EmptyTextGuard::Enforce);
        let thinking: ChatDelta = serde_json::from_value(serde_json::json!({
            "reasoning_content": "deep reasoning consumes the whole budget"
        }))
        .unwrap();
        let _ = sm.process_delta(&thinking, None);
        let events = sm.finalize(Some("length"), Some(64000), None);
        assert_eq!(
            events.len(),
            1,
            "enforce must emit only the error frame: {events:?}"
        );
        match &events[0] {
            SseEvent::Error { error } => {
                assert!(
                    error.message.contains("finish_reason=length"),
                    "must report length terminal: {}",
                    error.message
                );
            }
            other => panic!("expected SseEvent::Error, got {other:?}"),
        }
    }

    #[test]
    fn empty_text_guard_enforce_with_text_forwards_normally() {
        // Normal text response must be untouched by the guard.
        let mut sm = SseStateMachine::new(false, "reasoning_content".into(), vec![], None);
        sm.set_empty_text_guard(EmptyTextGuard::Enforce);
        let text: ChatDelta = serde_json::from_value(serde_json::json!({
            "content": "hello world"
        }))
        .unwrap();
        let _ = sm.process_delta(&text, None);
        let events = sm.finalize(Some("stop"), Some(10), None);
        assert!(
            matches!(events.last(), Some(SseEvent::MessageStop)),
            "text response must end with message_stop: {events:?}"
        );
        assert!(!events.iter().any(|e| matches!(e, SseEvent::Error { .. })));
    }

    #[test]
    fn empty_text_guard_enforce_with_tool_use_forwards_normally() {
        // Pure tool_use (no text) is a legitimate response and must be allowed.
        let mut sm = SseStateMachine::new(false, "reasoning_content".into(), vec![], None);
        sm.set_empty_text_guard(EmptyTextGuard::Enforce);
        let tool: ChatDelta = serde_json::from_value(serde_json::json!({
            "tool_calls": [{
                "index": 0, "id": "call_1", "type": "function",
                "function": {"name": "get_weather", "arguments": "{}"}
            }]
        }))
        .unwrap();
        let _ = sm.process_delta(&tool, None);
        let events = sm.finalize(Some("tool_calls"), Some(10), None);
        assert!(
            matches!(events.last(), Some(SseEvent::MessageStop)),
            "tool_use response must end with message_stop: {events:?}"
        );
        assert!(!events.iter().any(|e| matches!(e, SseEvent::Error { .. })));
    }

    #[test]
    fn empty_text_guard_off_and_warn_forward_empty_unchanged() {
        // Off and Warn must NOT change the wire — empty response forwarded as-is.
        for guard in [EmptyTextGuard::Off, EmptyTextGuard::Warn] {
            let mut sm = SseStateMachine::new(false, "reasoning_content".into(), vec![], None);
            sm.set_empty_text_guard(guard);
            let events = sm.finalize(Some("stop"), Some(20), None);
            assert!(
                matches!(events.last(), Some(SseEvent::MessageStop)),
                "{guard:?} must forward empty response with message_stop: {events:?}"
            );
            assert!(!events.iter().any(|e| matches!(e, SseEvent::Error { .. })));
        }
    }

    #[test]
    fn empty_text_guard_whitespace_only_text_counts_as_empty() {
        // Whitespace-only content must be treated as empty (matches CC's trim
        // on extraction), so the guard still fires in Enforce mode.
        let mut sm = SseStateMachine::new(false, "reasoning_content".into(), vec![], None);
        sm.set_empty_text_guard(EmptyTextGuard::Enforce);
        let ws: ChatDelta = serde_json::from_value(serde_json::json!({
            "content": "   \n\t  "
        }))
        .unwrap();
        let _ = sm.process_delta(&ws, None);
        let events = sm.finalize(Some("stop"), Some(5), None);
        assert!(
            matches!(events[0], SseEvent::Error { .. }),
            "whitespace-only must be treated as empty in enforce: {events:?}"
        );
    }
}
