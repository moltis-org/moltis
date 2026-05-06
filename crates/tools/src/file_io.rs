//! Shared file-reading logic for tools that read from host or sandbox filesystems.
//!
//! Used by `send_image` and `send_document` to read files consistently,
//! with sandbox routing and size validation.

use {std::sync::Arc, tracing::warn};

use crate::{
    Result,
    error::Error,
    sandbox::{
        SandboxRouter,
        file_system::{SandboxReadResult, sandbox_file_system_for_session},
    },
};

/// 20 MB — Telegram's maximum file upload size.
pub const MAX_FILE_SIZE: u64 = 20 * 1024 * 1024;

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
    let sandbox_fs = sandbox_file_system_for_session(router, session_key).await?;
    let read_result = sandbox_fs
        .read_file(path, MAX_FILE_SIZE)
        .await
        .map_err(|error| Error::message(format!("cannot access '{path}' in sandbox: {error}")))?;
    let bytes = match read_result {
        SandboxReadResult::Ok(bytes) => bytes,
        SandboxReadResult::NotFound => {
            return Err(Error::message(format!(
                "cannot access '{path}' in sandbox: file does not exist"
            )));
        },
        SandboxReadResult::PermissionDenied => {
            return Err(Error::message(format!(
                "cannot access '{path}' in sandbox: insufficient permissions to access file"
            )));
        },
        SandboxReadResult::NotRegularFile => {
            return Err(Error::message(format!(
                "cannot access '{path}' in sandbox: path is not a regular file"
            )));
        },
        SandboxReadResult::TooLarge(size) => {
            return Err(Error::message(format!(
                "file is too large ({:.1} MB) — maximum is {:.0} MB",
                size as f64 / (1024.0 * 1024.0),
                MAX_FILE_SIZE as f64 / (1024.0 * 1024.0),
            )));
        },
    };

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
    crate::sandbox::file_system::shell_single_quote(input)
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
