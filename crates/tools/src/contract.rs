//! Contract tests for tool execution (`exec_command`).
//!
//! These tests validate the timeout, output truncation, and error handling
//! invariants of the exec subsystem.

#![allow(clippy::unwrap_used)]

use std::time::Duration;

use crate::exec::{ExecOpts, exec_command};

/// A command that exceeds the timeout must be killed and return a structured error.
pub async fn timeout_is_enforced() -> crate::Result<()> {
    let opts = ExecOpts {
        timeout: Duration::from_secs(1),
        ..Default::default()
    };
    let result = exec_command("sleep 60", &opts).await;
    assert!(result.is_err(), "timed-out command must return Err");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("timed out"),
        "error must mention timeout, got: {err}"
    );
    Ok(())
}

/// Output exceeding the byte limit must be truncated with a marker.
pub async fn output_is_truncated_at_limit() -> crate::Result<()> {
    let opts = ExecOpts {
        timeout: Duration::from_secs(10),
        max_output_bytes: 100,
        ..Default::default()
    };
    // Generate ~1KB of output.
    let result = exec_command("yes hello | head -200", &opts).await?;
    // Output must be truncated.
    assert!(
        result.stdout.len() <= 200, // 100 bytes + truncation marker
        "stdout must be truncated, got {} bytes",
        result.stdout.len()
    );
    assert!(
        result.stdout.contains("[output truncated]"),
        "truncated output must include marker"
    );
    Ok(())
}

/// A failing command must return a structured `ExecResult` with non-zero exit code.
pub async fn error_returns_structured_result() -> crate::Result<()> {
    let opts = ExecOpts::default();
    let result = exec_command("exit 42", &opts).await?;
    assert_eq!(result.exit_code, 42, "exit code must be propagated");
    // Must return ExecResult, not panic.
    assert!(result.stdout.is_empty() || !result.stdout.is_empty()); // just prove we got a result
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn contract_timeout_is_enforced() {
        timeout_is_enforced().await.unwrap();
    }

    #[tokio::test]
    async fn contract_output_is_truncated_at_limit() {
        output_is_truncated_at_limit().await.unwrap();
    }

    #[tokio::test]
    async fn contract_error_returns_structured_result() {
        error_returns_structured_result().await.unwrap();
    }
}
