//! `moltis acp` — serve Moltis to an ACP client over stdio.
//!
//! Any harness that drives ACP agents (Zed, `buzz-acp`, a bespoke runner) spawns
//! the agent as a subprocess and speaks JSON-RPC over its stdin/stdout. One
//! client per process, matching how every ACP harness works.
//!
//! **stdout is the wire.** Callers must have redirected logging to stderr before
//! reaching this module — see `acp_reserves_stdout` in `main.rs`, which is what
//! flips the tracing writer.

use std::sync::Arc;

use {clap::Args, moltis_acp::AcpBackend};

#[derive(Args, Debug)]
pub struct AcpArgs {
    /// Serve a built-in echo agent instead of a real Moltis session.
    ///
    /// Useful for checking a client's handshake end to end without involving
    /// providers, sessions, or tools.
    #[arg(long)]
    pub echo: bool,
}

/// Resolves the backend to serve and runs the protocol loop until the client
/// disconnects.
pub async fn handle_acp(args: AcpArgs) -> anyhow::Result<()> {
    let backend = resolve_backend(&args)?;
    moltis_acp::run_stdio(backend).await
}

fn resolve_backend(args: &AcpArgs) -> anyhow::Result<Arc<dyn AcpBackend>> {
    if args.echo {
        return Ok(Arc::new(moltis_acp::EchoBackend::new()));
    }
    // Refusing is better than silently echoing: a client wired up to this
    // expecting real turns would otherwise look like it was working.
    Err(anyhow::anyhow!(
        "serving real Moltis turns over ACP is not wired up yet; run `moltis acp --echo` to \
         verify a client's handshake in the meantime"
    ))
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echo_flag_selects_the_echo_backend() {
        let backend = resolve_backend(&AcpArgs { echo: true }).expect("echo backend");
        assert!(!backend.capabilities().load_session);
    }

    #[test]
    fn without_echo_the_command_refuses_rather_than_pretending() {
        // `expect_err` would need `Arc<dyn AcpBackend>: Debug`, which is not
        // worth forcing onto every implementor.
        let error = resolve_backend(&AcpArgs { echo: false })
            .err()
            .expect("real turns are not implemented yet");
        assert!(
            error.to_string().contains("--echo"),
            "the error should point at the working alternative: {error}"
        );
    }
}
