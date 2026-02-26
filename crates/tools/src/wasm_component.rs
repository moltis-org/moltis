#[cfg(feature = "wasm")]
pub mod pure_tool {
    wasmtime::component::bindgen!({
        path: "../../wit",
        world: "pure-tool",
    });
}

#[cfg(feature = "wasm")]
pub mod http_tool {
    wasmtime::component::bindgen!({
        path: "../../wit",
        world: "http-tool",
    });
}

#[cfg(feature = "wasm")]
pub type PureToolValue = pure_tool::moltis::tool::types::ToolValue;

#[cfg(feature = "wasm")]
#[must_use]
pub fn marshal_tool_result(value: PureToolValue) -> serde_json::Value {
    match value {
        PureToolValue::Text(text) => serde_json::Value::String(text),
        PureToolValue::Number(number) => serde_json::Number::from_f64(number)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        PureToolValue::Integer(integer) => serde_json::Value::Number(integer.into()),
        PureToolValue::Boolean(boolean) => serde_json::Value::Bool(boolean),
        PureToolValue::Json(json) => match serde_json::from_str::<serde_json::Value>(&json) {
            Ok(parsed) => parsed,
            Err(_) => serde_json::Value::String(json),
        },
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(all(test, feature = "wasm"))]
mod tests {
    use super::{PureToolValue, marshal_tool_result};

    #[test]
    fn marshal_tool_result_text() {
        let value = marshal_tool_result(PureToolValue::Text("hello".to_string()));
        assert_eq!(value, serde_json::json!("hello"));
    }

    #[test]
    fn marshal_tool_result_number() {
        let value = marshal_tool_result(PureToolValue::Number(12.5));
        assert_eq!(value, serde_json::json!(12.5));
    }

    #[test]
    fn marshal_tool_result_integer() {
        let value = marshal_tool_result(PureToolValue::Integer(-42));
        assert_eq!(value, serde_json::json!(-42));
    }

    #[test]
    fn marshal_tool_result_boolean() {
        let value = marshal_tool_result(PureToolValue::Boolean(true));
        assert_eq!(value, serde_json::json!(true));
    }

    #[test]
    fn marshal_tool_result_json() {
        let value = marshal_tool_result(PureToolValue::Json("{\"k\":\"v\"}".to_string()));
        assert_eq!(value, serde_json::json!({"k": "v"}));
    }
}
