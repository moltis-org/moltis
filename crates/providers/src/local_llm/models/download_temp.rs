use std::path::{Path, PathBuf};

use {
    anyhow::Context,
    cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt},
    cap_std::fs::{Dir, OpenOptions},
    tokio::io::AsyncWriteExt,
};

pub(super) struct TemporaryDownload {
    dir: Dir,
    path: PathBuf,
    file: Option<tokio::fs::File>,
    cleanup_armed: bool,
}

impl TemporaryDownload {
    pub(super) fn create(dir: &Dir, destination: &Path) -> anyhow::Result<Self> {
        let path = temporary_path(destination)?;
        Self::create_at(dir, path)
    }

    fn create_at(dir: &Dir, path: PathBuf) -> anyhow::Result<Self> {
        let dir = dir.try_clone().context("cloning download directory")?;
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No);
        let file = dir
            .open_with(&path, &options)
            .context("creating temporary download file")?;

        Ok(Self {
            dir,
            path,
            file: Some(tokio::fs::File::from_std(file.into_std())),
            cleanup_armed: true,
        })
    }

    pub(super) async fn write_all(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        let Some(file) = self.file.as_mut() else {
            return Err(std::io::Error::other("temporary download file is closed"));
        };
        file.write_all(bytes).await
    }

    pub(super) async fn flush(&mut self) -> std::io::Result<()> {
        let Some(file) = self.file.as_mut() else {
            return Err(std::io::Error::other("temporary download file is closed"));
        };
        file.flush().await
    }

    pub(super) fn rename_to(mut self, destination: &Path) -> anyhow::Result<()> {
        drop(self.file.take());
        self.dir
            .rename(&self.path, &self.dir, destination)
            .context("renaming downloaded file")?;
        self.cleanup_armed = false;
        Ok(())
    }

    #[cfg(test)]
    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryDownload {
    fn drop(&mut self) {
        drop(self.file.take());
        if !self.cleanup_armed {
            return;
        }

        match self.dir.remove_file(&self.path) {
            Ok(()) => {},
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => {
                tracing::warn!(%error, path = ?self.path, "failed to clean up temporary download")
            },
        }
    }
}

fn temporary_path(destination: &Path) -> anyhow::Result<PathBuf> {
    let file_name = destination
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("download destination has no filename"))?;
    let mut temp_name = file_name.to_os_string();
    temp_name.push(format!(".{}.tmp", uuid::Uuid::new_v4().simple()));
    Ok(destination.with_file_name(temp_name))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn test_dir() -> (tempfile::TempDir, Dir) {
        let temp_dir = tempfile::tempdir().unwrap();
        let dir = Dir::open_ambient_dir(temp_dir.path(), cap_std::ambient_authority()).unwrap();
        (temp_dir, dir)
    }

    #[test]
    fn temporary_download_path_retains_full_destination_name() {
        let temp = temporary_path(Path::new("model.safetensors.index.json")).unwrap();
        let name = temp.file_name().unwrap().to_str().unwrap();

        assert!(name.starts_with("model.safetensors.index.json."));
        assert!(name.ends_with(".tmp"));
    }

    #[test]
    fn temporary_download_paths_are_unique() {
        let destination = Path::new("model.safetensors");

        assert_ne!(
            temporary_path(destination).unwrap(),
            temporary_path(destination).unwrap()
        );
    }

    #[tokio::test]
    async fn dropping_download_closes_and_removes_only_its_temporary_file() {
        let (temp_dir, dir) = test_dir();
        let destination = Path::new("model.safetensors");
        let mut first = TemporaryDownload::create(&dir, destination).unwrap();
        let second = TemporaryDownload::create(&dir, destination).unwrap();
        let first_path = first.path().to_path_buf();
        let second_path = second.path().to_path_buf();
        first.write_all(b"partial").await.unwrap();
        first.flush().await.unwrap();

        drop(first);

        assert!(!temp_dir.path().join(first_path).exists());
        assert!(temp_dir.path().join(&second_path).exists());
        drop(second);
        assert!(!temp_dir.path().join(second_path).exists());
    }

    #[tokio::test]
    async fn cancelling_owner_future_removes_temporary_file() {
        let (temp_dir, dir) = test_dir();
        let (path_sender, path_receiver) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let mut download =
                TemporaryDownload::create(&dir, Path::new("model.safetensors")).unwrap();
            download.write_all(b"partial").await.unwrap();
            download.flush().await.unwrap();
            path_sender.send(download.path().to_path_buf()).unwrap();
            std::future::pending::<()>().await;
            drop(download);
        });
        let path = path_receiver.await.unwrap();

        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());

        assert!(!temp_dir.path().join(path).exists());
    }

    #[tokio::test]
    async fn successful_rename_disarms_cleanup() {
        let (temp_dir, dir) = test_dir();
        let destination = Path::new("model.safetensors");
        let mut download = TemporaryDownload::create(&dir, destination).unwrap();
        let temp_path = download.path().to_path_buf();
        download.write_all(b"complete").await.unwrap();
        download.flush().await.unwrap();

        download.rename_to(destination).unwrap();

        assert!(!temp_dir.path().join(temp_path).exists());
        assert_eq!(
            std::fs::read(temp_dir.path().join(destination)).unwrap(),
            b"complete"
        );
    }

    #[test]
    fn failed_rename_rearms_cleanup_after_closing_file() {
        let (temp_dir, dir) = test_dir();
        let destination = Path::new("model.safetensors");
        let download = TemporaryDownload::create(&dir, destination).unwrap();
        let temp_path = download.path().to_path_buf();

        assert!(
            download
                .rename_to(Path::new("missing/model.safetensors"))
                .is_err()
        );
        assert!(!temp_dir.path().join(temp_path).exists());
    }

    #[cfg(unix)]
    #[test]
    fn creation_rejects_symlink_temporary_path() {
        let (temp_dir, dir) = test_dir();
        let outside = temp_dir.path().join("outside");
        let path = PathBuf::from("model.tmp");
        std::fs::write(&outside, b"outside").unwrap();
        std::os::unix::fs::symlink(&outside, temp_dir.path().join(&path)).unwrap();

        assert!(TemporaryDownload::create_at(&dir, path).is_err());
        assert_eq!(std::fs::read(outside).unwrap(), b"outside");
    }
}
