use std::{io, process::ExitStatus};

use {
    process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop},
    tokio::process::{ChildStderr, ChildStdin, ChildStdout, Command},
};

/// Owns a spawned process and all descendants that remain in its OS process
/// group (Unix) or Job Object (Windows).
#[derive(Debug)]
pub struct OwnedProcessTree {
    child: Box<dyn ChildWrapper>,
    terminated: bool,
}

impl OwnedProcessTree {
    pub fn spawn(command: Command) -> io::Result<Self> {
        let mut command = CommandWrap::from(command);
        command.wrap(KillOnDrop);
        #[cfg(unix)]
        command.wrap(process_wrap::tokio::ProcessGroup::leader());
        #[cfg(windows)]
        command.wrap(process_wrap::tokio::JobObject);

        Ok(Self {
            child: command.spawn()?,
            terminated: false,
        })
    }

    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.child.stdin().take()
    }

    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.stdout().take()
    }

    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.child.stderr().take()
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    pub async fn wait(&mut self) -> io::Result<ExitStatus> {
        let status = self.child.wait().await?;
        self.terminated = true;
        Ok(status)
    }

    pub async fn kill(&mut self) -> io::Result<()> {
        Box::into_pin(self.child.kill()).await?;
        self.terminated = true;
        Ok(())
    }
}

impl Drop for OwnedProcessTree {
    fn drop(&mut self) {
        if !self.terminated {
            let _ = self.child.start_kill();
        }
    }
}

#[cfg(all(test, unix))]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::{process::Stdio, time::Duration};

    use tokio::io::{AsyncBufReadExt, BufReader};

    use super::*;

    fn descendant_command(marker: &std::path::Path) -> Command {
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg(format!(
                "(sleep 1; touch '{}') & echo ready; wait",
                marker.display()
            ))
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .stdin(Stdio::null());
        command
    }

    async fn wait_until_ready(tree: &mut OwnedProcessTree) {
        let stdout = tree.take_stdout().unwrap();
        let mut line = String::new();
        BufReader::new(stdout).read_line(&mut line).await.unwrap();
        assert_eq!(line.trim(), "ready");
    }

    #[tokio::test]
    async fn drop_kills_descendants() {
        let temp_dir = tempfile::tempdir().unwrap();
        let marker = temp_dir.path().join("drop-marker");
        let mut tree = OwnedProcessTree::spawn(descendant_command(&marker)).unwrap();
        wait_until_ready(&mut tree).await;

        drop(tree);
        tokio::time::sleep(Duration::from_millis(1100)).await;
        assert!(!marker.exists(), "descendant survived process-tree drop");
    }

    #[tokio::test]
    async fn explicit_kill_terminates_descendants() {
        let temp_dir = tempfile::tempdir().unwrap();
        let marker = temp_dir.path().join("kill-marker");
        let mut tree = OwnedProcessTree::spawn(descendant_command(&marker)).unwrap();
        wait_until_ready(&mut tree).await;

        tree.kill().await.unwrap();
        tokio::time::sleep(Duration::from_millis(1100)).await;
        assert!(!marker.exists(), "descendant survived process-tree kill");
    }
}
