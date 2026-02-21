//! Custom GraphQL scalars.

use async_graphql::{InputValueError, InputValueResult, Scalar, ScalarType, Value};

/// A JSON scalar that passes through arbitrary `serde_json::Value` data.
///
/// Used for dynamic/untyped fields where the exact shape varies at runtime
/// (e.g. config values, dynamic params).
pub struct Json(pub serde_json::Value);

#[Scalar]
impl ScalarType for Json {
    fn parse(value: Value) -> InputValueResult<Self> {
        let json = gql_value_to_json(value).map_err(InputValueError::custom)?;
        Ok(Json(json))
    }

    fn to_value(&self) -> Value {
        json_to_gql_value(&self.0)
    }
}

fn gql_value_to_json(v: Value) -> Result<serde_json::Value, String> {
    match v {
        Value::Null => Ok(serde_json::Value::Null),
        Value::Number(n) => serde_json::to_value(n).map_err(|e| e.to_string()),
        Value::String(s) => Ok(serde_json::Value::String(s)),
        Value::Boolean(b) => Ok(serde_json::Value::Bool(b)),
        Value::List(l) => {
            let items: Result<Vec<serde_json::Value>, _> =
                l.into_iter().map(gql_value_to_json).collect();
            Ok(serde_json::Value::Array(items?))
        },
        Value::Object(m) => {
            let map: Result<serde_json::Map<String, serde_json::Value>, _> = m
                .into_iter()
                .map(|(k, v)| gql_value_to_json(v).map(|jv| (k.to_string(), jv)))
                .collect();
            Ok(serde_json::Value::Object(map?))
        },
        _ => Err("unsupported value type".into()),
    }
}

fn json_to_gql_value(v: &serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Boolean(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Number(i.into())
            } else if let Some(f) = n.as_f64() {
                Value::Number(async_graphql::Number::from_f64(f).unwrap_or_else(|| 0i32.into()))
            } else {
                Value::Null
            }
        },
        serde_json::Value::String(s) => Value::String(s.clone()),
        serde_json::Value::Array(a) => Value::List(a.iter().map(json_to_gql_value).collect()),
        serde_json::Value::Object(m) => {
            let map: async_graphql::indexmap::IndexMap<async_graphql::Name, Value> = m
                .iter()
                .map(|(k, v)| (async_graphql::Name::new(k), json_to_gql_value(v)))
                .collect();
            Value::Object(map)
        },
    }
}
