/// Escape a value for use inside a zvec filter string literal.
///
/// zvec's filter grammar uses C-style backslash escapes inside single-quoted
/// strings: `\<any>` is an escaped pair. A literal single quote is escaped as
/// `\'` (NOT SQL-style `''` doubling). Backslashes are doubled first so they
/// don't consume the following character.
pub(crate) fn escape_filter_value(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}
