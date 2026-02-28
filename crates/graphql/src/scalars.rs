//! Custom GraphQL scalars.

use async_graphql::{InputValueError, InputValueResult, Scalar, ScalarType, Value};

#[derive(Debug, thiserror::Error)]
enum ScalarError {
    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error("unsupported value type")]
    UnsupportedValueType,
}

/// A JSON scalar that passes through arbitrary `serde_json::Value` data.
///
/// Used for dynamic/untyped fields where the exact shape varies at runtime
/// (e.g. config values, dynamic params).
pub struct Json(pub serde_json::Value);

#[Scalar(name = "JSON")]
impl ScalarType for Json {
    fn parse(value: Value) -> InputValueResult<Self> {
        let json = gql_value_to_json(value).map_err(InputValueError::custom)?;
        Ok(Json(json))
    }

    fn to_value(&self) -> Value {
        json_to_gql_value(&self.0)
    }
}

fn gql_value_to_json(v: Value) -> Result<serde_json::Value, ScalarError> {
    match v {
        Value::Null => Ok(serde_json::Value::Null),
        Value::Number(n) => Ok(serde_json::to_value(n)?),
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
        _ => Err(ScalarError::UnsupportedValueType),
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

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use {super::*, async_graphql::Name};

    #[test]
    fn json_scalar_round_trips_structures() {
        let input = Value::Object(
            [
                (Name::new("a"), Value::Number(1.into())),
                (Name::new("b"), Value::Boolean(true)),
                (
                    Name::new("c"),
                    Value::List(vec![Value::String("x".into()), Value::Null]),
                ),
            ]
            .into_iter()
            .collect(),
        );

        let parsed = Json::parse(input).expect("parse");
        let out = parsed.to_value();
        let json = gql_value_to_json(out).expect("to json");
        assert_eq!(json["a"], 1);
        assert_eq!(json["b"], true);
        assert_eq!(json["c"][0], "x");
    }

    #[test]
    fn json_scalar_rejects_unsupported_values() {
        let unsupported = Value::Enum(Name::new("SOMETHING"));
        let err = Json::parse(unsupported).expect_err("expected parse error");
        assert!(format!("{err:?}").contains("unsupported value type"));
    }

    #[test]
    fn json_scalar_handles_null_numbers_and_arrays() {
        let parsed = Json::parse(Value::List(vec![
            Value::Null,
            Value::Number(42.into()),
            Value::Number(async_graphql::Number::from_f64(1.5).expect("valid float")),
        ]))
        .expect("parse");
        let out = parsed.to_value();
        let json = gql_value_to_json(out).expect("to json");
        assert!(json.is_array());
        assert_eq!(json[0], serde_json::Value::Null);
        assert_eq!(json[1], 42);
    }
}
