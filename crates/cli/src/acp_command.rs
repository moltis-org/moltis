//! `moltis acp` — serve Moltis to an ACP client over stdio.
//!
//! Any harness that drives ACP agents (Zed, `buzz-acp`, a bespoke runner) spawns
//! the agent as a subprocess and speaks JSON-RPC over its stdin/stdout. One
//! client per process, matching how every ACP harness works.
//!
//! **stdout is the wire.** Callers must have redirected logging to stderr before
//! reaching this module — see `acp_reserves_stdout` in `main.rs`, which is what
//! flips the tracing writer.

use std::{path::PathBuf, sync::Arc};

use {clap::Args, moltis_acp::AcpBackend};

use crate::acp_backend::MoltisBackend;

#[derive(Args, Debug)]
pub struct AcpArgs {
    /// Serve a built-in echo agent instead of a real Moltis session.
    ///
    /// Useful for checking a client's handshake end to end without involving
    /// providers, sessions, or tools.
    #[arg(long)]
    pub echo: bool,

    /// Override the config directory.
    #[arg(long)]
    pub config_dir: Option<PathBuf>,

    /// Override the data directory.
    #[arg(long)]
    pub data_dir: Option<PathBuf>,
}

/// Resolves the backend to serve and runs the protocol loop until the client
/// disconnects.
pub async fn handle_acp(args: AcpArgs) -> anyhow::Result<()> {
    let backend = resolve_backend(&args).await?;
    moltis_acp::run_stdio(backend).await
}

async fn resolve_backend(args: &AcpArgs) -> anyhow::Result<Arc<dyn AcpBackend>> {
    if args.echo {
        return Ok(Arc::new(moltis_acp::EchoBackend::new()));
    }
    Ok(Arc::new(MoltisBackend::new(boot_core(args).await?)))
}

/// Boots the Moltis stack in-process.
///
/// `prepare_gateway_core_with_profile` is the transport-agnostic half of startup: it wires
/// providers, sessions, memory and tools but binds no socket, so serving ACP
/// does not stand up an HTTP listener or collide with a running gateway on a
/// port. The bind address it takes is only recorded for OAuth callbacks.
async fn boot_core(args: &AcpArgs) -> anyhow::Result<moltis_gateway::server::PreparedGatewayCore> {
    let core = moltis_gateway::server::prepare_gateway_core_with_profile(
        "127.0.0.1",
        0,
        true,
        None,
        args.config_dir.clone(),
        args.data_dir.clone(),
        None,
        None,
        None,
        moltis_gateway::server::CoreStartupProfile::Headless,
    )
    .await?;
    Ok(core)
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    fn args(echo: bool) -> AcpArgs {
        AcpArgs {
            echo,
            config_dir: None,
            data_dir: None,
        }
    }

    #[tokio::test]
    async fn echo_flag_selects_the_echo_backend() {
        // Resolving the echo backend must not boot the Moltis stack: the flag
        // exists to check a client's handshake without providers or databases.
        let backend = resolve_backend(&args(true)).await.expect("echo backend");
        assert!(!backend.capabilities().load_session);
    }
}
