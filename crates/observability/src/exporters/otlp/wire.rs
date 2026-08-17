//! OTLP/HTTP JSON wire types.
//!
//! Hand-written rather than generated from protobuf: the JSON encoding is a
//! small, stable subset and hand-rolling it avoids pulling `prost`, `tonic` and
//! a build-time protoc dependency into the workspace for one request shape.
//!
//! The one sharp edge in OTLP/JSON is that 64-bit integers are encoded as
//! **decimal strings**, not JSON numbers, because JSON numbers cannot carry the
//! full int64 range. Timestamps and `intValue` therefore serialize as strings.

use serde::Serialize;

/// Top-level OTLP trace export request.
#[derive(Debug, Clone, Serialize)]
pub struct ExportTraceServiceRequest {
    /// Spans grouped by emitting resource.
    #[serde(rename = "resourceSpans")]
    pub resource_spans: Vec<ResourceSpans>,
}

/// Spans from one resource (one service instance).
#[derive(Debug, Clone, Serialize)]
pub struct ResourceSpans {
    /// Attributes describing the emitting service.
    pub resource: Resource,
    /// Spans grouped by instrumentation scope.
    #[serde(rename = "scopeSpans")]
    pub scope_spans: Vec<ScopeSpans>,
}

/// The emitting service.
#[derive(Debug, Clone, Serialize)]
pub struct Resource {
    /// Resource-level attributes.
    pub attributes: Vec<KeyValue>,
}

/// Spans from one instrumentation scope.
#[derive(Debug, Clone, Serialize)]
pub struct ScopeSpans {
    /// The instrumentation library.
    pub scope: InstrumentationScope,
    /// The spans themselves.
    pub spans: Vec<Span>,
}

/// Instrumentation library identity.
#[derive(Debug, Clone, Serialize)]
pub struct InstrumentationScope {
    /// Library name.
    pub name: String,
    /// Library version.
    pub version: String,
}

/// A single span.
#[derive(Debug, Clone, Serialize)]
pub struct Span {
    /// 32-character lowercase hex trace id.
    #[serde(rename = "traceId")]
    pub trace_id: String,
    /// 16-character lowercase hex span id.
    #[serde(rename = "spanId")]
    pub span_id: String,
    /// Parent span id, omitted for a root span.
    #[serde(rename = "parentSpanId", skip_serializing_if = "Option::is_none")]
    pub parent_span_id: Option<String>,
    /// Span name.
    pub name: String,
    /// Span kind. `1` is INTERNAL, which is correct for in-process work.
    pub kind: i32,
    /// Start time in nanoseconds since the Unix epoch, as a decimal string.
    #[serde(rename = "startTimeUnixNano")]
    pub start_time_unix_nano: String,
    /// End time in nanoseconds since the Unix epoch, as a decimal string.
    #[serde(rename = "endTimeUnixNano")]
    pub end_time_unix_nano: String,
    /// Span attributes.
    pub attributes: Vec<KeyValue>,
    /// Span status.
    pub status: Status,
}

/// Span kind: in-process work.
pub const SPAN_KIND_INTERNAL: i32 = 1;

/// Span completion status.
#[derive(Debug, Clone, Serialize)]
pub struct Status {
    /// `0` unset, `1` OK, `2` ERROR.
    pub code: i32,
    /// Human-readable detail, present only on error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Status code: no explicit status.
pub const STATUS_UNSET: i32 = 0;
/// Status code: failed.
pub const STATUS_ERROR: i32 = 2;

impl Status {
    /// A status carrying no signal.
    #[must_use]
    pub const fn unset() -> Self {
        Self {
            code: STATUS_UNSET,
            message: None,
        }
    }

    /// A failure status with detail.
    #[must_use]
    pub fn error(message: Option<String>) -> Self {
        Self {
            code: STATUS_ERROR,
            message,
        }
    }
}

/// One attribute.
#[derive(Debug, Clone, Serialize)]
pub struct KeyValue {
    /// Attribute name.
    pub key: String,
    /// Attribute value.
    pub value: AnyValue,
}

impl KeyValue {
    /// String-valued attribute.
    #[must_use]
    pub fn string(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: AnyValue::String {
                string_value: value.into(),
            },
        }
    }

    /// Integer-valued attribute.
    #[must_use]
    pub fn int(key: impl Into<String>, value: i64) -> Self {
        Self {
            key: key.into(),
            value: AnyValue::Int {
                int_value: value.to_string(),
            },
        }
    }

    /// Float-valued attribute.
    #[must_use]
    pub fn double(key: impl Into<String>, value: f64) -> Self {
        Self {
            key: key.into(),
            value: AnyValue::Double {
                double_value: value,
            },
        }
    }

    /// Boolean-valued attribute.
    #[must_use]
    pub fn bool(key: impl Into<String>, value: bool) -> Self {
        Self {
            key: key.into(),
            value: AnyValue::Bool { bool_value: value },
        }
    }

    /// String-array-valued attribute.
    #[must_use]
    pub fn string_array(key: impl Into<String>, values: &[String]) -> Self {
        Self {
            key: key.into(),
            value: AnyValue::Array {
                array_value: ArrayValue {
                    values: values
                        .iter()
                        .map(|v| AnyValue::String {
                            string_value: v.clone(),
                        })
                        .collect(),
                },
            },
        }
    }
}

/// OTLP `AnyValue`.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum AnyValue {
    /// UTF-8 string.
    String {
        /// The string.
        #[serde(rename = "stringValue")]
        string_value: String,
    },
    /// 64-bit integer, encoded as a decimal string per OTLP/JSON.
    Int {
        /// The integer, decimal-encoded.
        #[serde(rename = "intValue")]
        int_value: String,
    },
    /// Double-precision float.
    Double {
        /// The float.
        #[serde(rename = "doubleValue")]
        double_value: f64,
    },
    /// Boolean.
    Bool {
        /// The boolean.
        #[serde(rename = "boolValue")]
        bool_value: bool,
    },
    /// Homogeneous array.
    Array {
        /// The array.
        #[serde(rename = "arrayValue")]
        array_value: ArrayValue,
    },
}

/// OTLP array value wrapper.
#[derive(Debug, Clone, Serialize)]
pub struct ArrayValue {
    /// Array elements.
    pub values: Vec<AnyValue>,
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn int_values_serialize_as_decimal_strings() {
        // OTLP/JSON requires int64 as a string; emitting a JSON number here
        // makes collectors reject or silently truncate large values.
        let kv = KeyValue::int("gen_ai.usage.input_tokens", 9_007_199_254_740_993);
        let json = serde_json::to_value(&kv).expect("serializable");

        assert_eq!(json["value"]["intValue"], "9007199254740993");
        assert!(json["value"]["intValue"].is_string());
    }

    #[test]
    fn string_values_serialize_under_string_value() {
        let kv = KeyValue::string("service.name", "moltis");
        let json = serde_json::to_value(&kv).expect("serializable");

        assert_eq!(json["key"], "service.name");
        assert_eq!(json["value"]["stringValue"], "moltis");
    }

    #[test]
    fn bool_and_double_use_their_own_slots() {
        let b = serde_json::to_value(KeyValue::bool("k", true)).expect("serializable");
        let d = serde_json::to_value(KeyValue::double("k", 0.5)).expect("serializable");

        assert_eq!(b["value"]["boolValue"], true);
        assert_eq!(d["value"]["doubleValue"], 0.5);
    }

    #[test]
    fn string_arrays_nest_under_array_value() {
        let kv = KeyValue::string_array("langfuse.trace.tags", &["a".into(), "b".into()]);
        let json = serde_json::to_value(&kv).expect("serializable");

        assert_eq!(json["value"]["arrayValue"]["values"][0]["stringValue"], "a");
        assert_eq!(json["value"]["arrayValue"]["values"][1]["stringValue"], "b");
    }

    #[test]
    fn unset_status_omits_the_message_field() {
        let json = serde_json::to_value(Status::unset()).expect("serializable");
        assert_eq!(json["code"], 0);
        assert!(json.get("message").is_none());
    }

    #[test]
    fn error_status_carries_code_two_and_detail() {
        let json = serde_json::to_value(Status::error(Some("boom".into()))).expect("serializable");
        assert_eq!(json["code"], 2);
        assert_eq!(json["message"], "boom");
    }

    #[test]
    fn root_span_omits_parent_span_id() {
        let span = Span {
            trace_id: "0".repeat(32),
            span_id: "0".repeat(16),
            parent_span_id: None,
            name: "root".into(),
            kind: SPAN_KIND_INTERNAL,
            start_time_unix_nano: "1".into(),
            end_time_unix_nano: "2".into(),
            attributes: Vec::new(),
            status: Status::unset(),
        };
        let json = serde_json::to_value(&span).expect("serializable");

        // A present-but-null parentSpanId makes some collectors treat the span
        // as a child of a missing parent, orphaning the trace.
        assert!(json.get("parentSpanId").is_none());
    }
}
