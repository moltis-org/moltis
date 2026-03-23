//! Shared file-reading logic for tools that read from host or sandbox filesystems.
//!
//! Used by `send_image` and `send_document` to read files consistently,
//! with sandbox routing and size validation.

use {
    base64::{Engine as _, engine::general_purpose::STANDARD as BASE64},
    std::{path::PathBuf, sync::Arc, time::Duration},
    tracing::warn,
};

use crate::{Result, error::Error, exec::ExecOpts, sandbox::SandboxRouter};

/// 20 MB — Telegram's maximum file upload size.
pub const MAX_FILE_SIZE: u64 = 20 * 1024 * 1024;

/// Enough for a 20 MB binary file encoded as base64 (~26.7 MB) plus margin.
pub const MAX_SANDBOX_OUTPUT_BYTES: usize = 32 * 1024 * 1024;

/// Prefix emitted by the sandbox script when the file exceeds the size limit.
pub const SANDBOX_TOO_LARGE_PREFIX: &str = "__MOLTIS_FILE_TOO_LARGE__:";

/// Read a file from the host filesystem with size validation.
pub async fn read_host_file(path: &str) -> Result<Vec<u8>> {
    let meta = tokio::fs::metadata(path)
        .await
        .map_err(|e| Error::message(format!("cannot access '{path}': {e}")))?;

    if !meta.is_file() {
        return Err(Error::message(format!("'{path}' is not a regular file")));
    }

    if meta.len() > MAX_FILE_SIZE {
        return Err(Error::message(format!(
            "file is too large ({:.1} MB) — maximum is {:.0} MB",
            meta.len() as f64 / (1024.0 * 1024.0),
            MAX_FILE_SIZE as f64 / (1024.0 * 1024.0),
        )));
    }

    let bytes = tokio::fs::read(path)
        .await
        .map_err(|e| Error::message(format!("failed to read '{path}': {e}")))?;

    // Post-read size guard against TOCTOU races.
    if bytes.len() as u64 > MAX_FILE_SIZE {
        return Err(Error::message(format!(
            "file is too large ({:.1} MB) — maximum is {:.0} MB",
            bytes.len() as f64 / (1024.0 * 1024.0),
            MAX_FILE_SIZE as f64 / (1024.0 * 1024.0),
        )));
    }

    Ok(bytes)
}

/// Read a file from a sandbox container, returning the raw bytes.
pub async fn read_sandbox_file(
    router: &SandboxRouter,
    session_key: &str,
    path: &str,
) -> Result<Vec<u8>> {
    let sandbox_id = router.sandbox_id_for(session_key);
    let image = router.resolve_image(session_key, None).await;
    let backend = router.backend();
    backend.ensure_ready(&sandbox_id, Some(&image)).await?;

    let quoted_path = shell_single_quote(path);
    let command = format!(
        "if [ ! -f {quoted_path} ]; then \
             echo \"path is not a regular file\" >&2; \
             exit 2; \
         fi; \
         size=$(wc -c < {quoted_path}); \
         if [ \"$size\" -gt {MAX_FILE_SIZE} ]; then \
             echo \"{SANDBOX_TOO_LARGE_PREFIX}$size\" >&2; \
             exit 3; \
         fi; \
         base64 < {quoted_path} | tr -d '\\n'"
    );

    let opts = ExecOpts {
        timeout: Duration::from_secs(30),
        max_output_bytes: MAX_SANDBOX_OUTPUT_BYTES,
        working_dir: Some(PathBuf::from("/home/sandbox")),
        env: Vec::new(),
    };

    let result = backend.exec(&sandbox_id, &command, &opts).await?;
    if result.exit_code != 0 {
        if let Some(size_str) = result
            .stderr
            .lines()
            .find_map(|line| line.strip_prefix(SANDBOX_TOO_LARGE_PREFIX))
            && let Ok(size) = size_str.trim().parse::<u64>()
        {
            return Err(Error::message(format!(
                "file is too large ({:.1} MB) — maximum is {:.0} MB",
                size as f64 / (1024.0 * 1024.0),
                MAX_FILE_SIZE as f64 / (1024.0 * 1024.0),
            )));
        }

        let detail = if !result.stderr.trim().is_empty() {
            result.stderr.trim().to_string()
        } else if !result.stdout.trim().is_empty() {
            result.stdout.trim().to_string()
        } else {
            format!("sandbox command failed with exit code {}", result.exit_code)
        };
        return Err(Error::message(format!(
            "cannot access '{path}' in sandbox: {detail}"
        )));
    }

    let bytes = BASE64
        .decode(result.stdout.trim())
        .map_err(|e| Error::message(format!("failed to decode sandbox file '{path}': {e}")))?;

    if bytes.len() as u64 > MAX_FILE_SIZE {
        return Err(Error::message(format!(
            "file is too large ({:.1} MB) — maximum is {:.0} MB",
            bytes.len() as f64 / (1024.0 * 1024.0),
            MAX_FILE_SIZE as f64 / (1024.0 * 1024.0),
        )));
    }

    Ok(bytes)
}

/// Read a file for a session, routing through sandbox if the session is sandboxed.
pub async fn read_file_for_session(
    sandbox_router: Option<&Arc<SandboxRouter>>,
    session_key: &str,
    path: &str,
    tool_name: &str,
) -> Result<Vec<u8>> {
    let Some(router) = sandbox_router else {
        return read_host_file(path).await;
    };

    if !router.is_sandboxed(session_key).await {
        return read_host_file(path).await;
    }

    match read_sandbox_file(router, session_key, path).await {
        Ok(bytes) => Ok(bytes),
        Err(error) => {
            warn!(
                session_key,
                path,
                error = %error,
                "{tool_name} failed to read from sandbox"
            );
            Err(error)
        },
    }
}

/// Escape a string for safe use inside single quotes in a POSIX shell.
pub fn shell_single_quote(input: &str) -> String {
    format!("'{}'", input.replace('\'', "'\\''"))
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, std::io::Write, tempfile};

    #[test]
    fn shell_single_quote_simple() {
        assert_eq!(shell_single_quote("hello"), "'hello'");
    }

    #[test]
    fn shell_single_quote_with_quotes() {
        assert_eq!(shell_single_quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn shell_single_quote_empty() {
        assert_eq!(shell_single_quote(""), "''");
    }

    #[tokio::test]
    async fn read_host_file_success() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(b"hello world").unwrap();

        let bytes = read_host_file(tmp.path().to_str().unwrap()).await.unwrap();
        assert_eq!(bytes, b"hello world");
    }

    #[tokio::test]
    async fn read_host_file_nonexistent() {
        let err = read_host_file("/tmp/does-not-exist-file-io-test-987654.bin")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("cannot access"));
    }

    #[tokio::test]
    async fn read_host_file_too_large() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.bin");
        let f = std::fs::File::create(&path).unwrap();
        f.set_len(MAX_FILE_SIZE + 1).unwrap();

        let err = read_host_file(path.to_str().unwrap()).await.unwrap_err();
        assert!(err.to_string().contains("too large"));
    }

    #[tokio::test]
    async fn read_host_file_not_regular() {
        let dir = tempfile::tempdir().unwrap();
        let err = read_host_file(dir.path().to_str().unwrap())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not a regular file"));
    }
}
