/// Escape a value for use inside a zvec filter string literal.
///
/// zvec's filter grammar is SQL-like: string literals are single-quoted and a
/// literal quote is escaped by doubling it (`'` → `''`), matching SQLite/SQL
/// conventions. Paths are expected to already be validated file paths; this
/// guard prevents quote-injection from breaking out of the literal.
pub(crate) fn escape_filter_value(s: &str) -> String {
    s.replace('\'', "''")
}
