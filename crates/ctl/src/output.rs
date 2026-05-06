//! Output formatting for `moltis-ctl`.

use serde_json::Value;

/// Print a JSON value to stdout. Compact by default, pretty with `--pretty`.
pub fn print_json(value: &Value, pretty: bool) {
    let s = if pretty {
        serde_json::to_string_pretty(value).unwrap_or_default()
    } else {
        serde_json::to_string(value).unwrap_or_default()
    };
    println!("{s}");
}
