use crate::{ConnectorError, Result};

pub const MAX_SEARCH_QUERY_LENGTH: usize = 512;
const MAX_SEARCH_TERMS: usize = 16;
const MAX_SEARCH_TERM_LENGTH: usize = 64;

pub(crate) fn fts_query(text: &str) -> Result<Option<String>> {
    if text.chars().count() > MAX_SEARCH_QUERY_LENGTH {
        return Err(ConnectorError::InvalidInput(format!(
            "search text must not exceed {MAX_SEARCH_QUERY_LENGTH} characters"
        )));
    }

    let terms = text
        .split(|character: char| !character.is_alphanumeric())
        .filter(|term| !term.is_empty())
        .take(MAX_SEARCH_TERMS)
        .map(|term| {
            term.chars()
                .take(MAX_SEARCH_TERM_LENGTH)
                .collect::<String>()
        })
        .map(|term| format!("\"{term}\""))
        .collect::<Vec<_>>();
    Ok((!terms.is_empty()).then(|| terms.join(" AND ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turns_special_characters_into_literal_terms() -> Result<()> {
        assert_eq!(
            fts_query("  Alice's (Q3) *** \"review\"  ")?.as_deref(),
            Some("\"Alice\" AND \"s\" AND \"Q3\" AND \"review\"")
        );
        assert_eq!(fts_query("***")?, None);
        Ok(())
    }

    #[test]
    fn rejects_oversized_queries() {
        assert!(matches!(
            fts_query(&"a".repeat(MAX_SEARCH_QUERY_LENGTH + 1)),
            Err(ConnectorError::InvalidInput(_))
        ));
    }
}
