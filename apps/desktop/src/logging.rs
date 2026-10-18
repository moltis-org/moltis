//! Log macros that work with or without the `tracing` feature. Without it a
//! call compiles to nothing but still uses the fields it names, so call sites
//! need no `cfg`. Only the `%field`/`?field` forms with a literal message are
//! supported.

#[cfg(feature = "tracing")]
pub(crate) use tracing::{debug, error, info, warn};

#[cfg(not(feature = "tracing"))]
macro_rules! discard {
    ($(%$display:ident,)* $(?$debug:ident,)* $message:literal) => {{
        $(let _ = &$display;)*
        $(let _ = &$debug;)*
    }};
}

#[cfg(not(feature = "tracing"))]
pub(crate) use {discard as debug, discard as error, discard as info, discard as warn};
