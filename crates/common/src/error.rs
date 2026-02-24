use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("{0}")]
    Message(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("internal error")]
    Other {
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

impl Error {
    #[must_use]
    pub fn message(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }

    #[must_use]
    pub fn other(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Other {
            source: Box::new(source),
        }
    }
}

impl FromMessage for Error {
    fn from_message(message: String) -> Self {
        Self::Message(message)
    }
}

pub type MoltisError = Error;
pub type Result<T> = std::result::Result<T, Error>;

// ── Shared context trait ────────────────────────────────────────────────────

/// Trait for error types that can be constructed from a plain message string.
///
/// Implement this for your crate's error type, then invoke [`impl_context!`]
/// in your error module to get `.context()` and `.with_context()` on `Result`
/// and `Option`.
pub trait FromMessage: Sized {
    fn from_message(message: String) -> Self;
}

/// Generate a crate-local `Context` trait with `.context()` and `.with_context()`
/// methods on `Result` and `Option`.
///
/// Invoke inside a module that defines `Error: FromMessage` and
/// `type Result<T> = std::result::Result<T, Error>`.
///
/// ```ignore
/// // in crates/foo/src/error.rs
/// moltis_common::impl_context!();
/// ```
#[macro_export]
macro_rules! impl_context {
    () => {
        pub trait Context<T> {
            fn context(self, context: impl Into<String>) -> Result<T>;
            fn with_context<C, F>(self, f: F) -> Result<T>
            where
                C: Into<String>,
                F: FnOnce() -> C;
        }

        impl<T, E: std::fmt::Display> Context<T> for std::result::Result<T, E> {
            fn context(self, context: impl Into<String>) -> Result<T> {
                let ctx = context.into();
                self.map_err(|source| {
                    <Error as $crate::FromMessage>::from_message(format!("{ctx}: {source}"))
                })
            }

            fn with_context<C, F>(self, f: F) -> Result<T>
            where
                C: Into<String>,
                F: FnOnce() -> C,
            {
                self.map_err(|source| {
                    let ctx = f().into();
                    <Error as $crate::FromMessage>::from_message(format!("{ctx}: {source}"))
                })
            }
        }

        impl<T> Context<T> for Option<T> {
            fn context(self, context: impl Into<String>) -> Result<T> {
                self.ok_or_else(|| <Error as $crate::FromMessage>::from_message(context.into()))
            }

            fn with_context<C, F>(self, f: F) -> Result<T>
            where
                C: Into<String>,
                F: FnOnce() -> C,
            {
                self.ok_or_else(|| <Error as $crate::FromMessage>::from_message(f().into()))
            }
        }
    };
}
