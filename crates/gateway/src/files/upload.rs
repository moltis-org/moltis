use {cap_std::fs::Dir, std::io, tokio::io::AsyncWriteExt};

#[cfg(windows)]
use uuid::Uuid;

use super::{FileEntry, FilesError, FilesResult, error::map_io, types::entry_metadata};

pub struct PendingUpload {
    dir: Dir,
    temp_name: String,
    destination_name: String,
    logical_path: String,
    file: Option<tokio::fs::File>,
    overwrite: bool,
    cleanup_armed: bool,
}

impl PendingUpload {
    pub(crate) fn new(
        dir: Dir,
        temp_name: String,
        destination_name: String,
        logical_path: String,
        file: tokio::fs::File,
        overwrite: bool,
    ) -> Self {
        Self {
            dir,
            temp_name,
            destination_name,
            logical_path,
            file: Some(file),
            overwrite,
            cleanup_armed: true,
        }
    }

    pub fn writer(&mut self) -> FilesResult<&mut tokio::fs::File> {
        self.file
            .as_mut()
            .ok_or_else(|| FilesError::Io(io::Error::other("upload writer is closed")))
    }

    pub async fn commit(mut self) -> FilesResult<FileEntry> {
        let Some(file) = self.file.as_mut() else {
            return Err(FilesError::Io(io::Error::other("upload writer is closed")));
        };
        file.flush().await.map_err(map_io)?;
        file.sync_all().await.map_err(map_io)?;
        drop(self.file.take());

        let destination_exists = match self.dir.symlink_metadata(&self.destination_name) {
            Ok(_) if !self.overwrite => return Err(FilesError::Conflict),
            Ok(metadata) if metadata.is_file() => true,
            Ok(_) => return Err(FilesError::UnsupportedEntry),
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => return Err(map_io(error)),
        };

        commit_temp_file(
            &self.dir,
            &self.temp_name,
            &self.destination_name,
            self.overwrite,
            destination_exists,
        )?;
        self.cleanup_armed = false;
        let metadata = self
            .dir
            .symlink_metadata(&self.destination_name)
            .map_err(map_io)?;
        Ok(FileEntry {
            name: self.destination_name.clone(),
            path: self.logical_path.clone(),
            metadata: entry_metadata(&metadata)?,
        })
    }
}

#[cfg(not(windows))]
fn commit_temp_file(
    dir: &Dir,
    temp_name: &str,
    destination_name: &str,
    overwrite: bool,
    _destination_exists: bool,
) -> FilesResult<()> {
    if !overwrite {
        dir.hard_link(temp_name, dir, destination_name)
            .map_err(map_io)?;
        return dir.remove_file(temp_name).map_err(map_io);
    }
    dir.rename(temp_name, dir, destination_name).map_err(map_io)
}

#[cfg(windows)]
fn commit_temp_file(
    dir: &Dir,
    temp_name: &str,
    destination_name: &str,
    overwrite: bool,
    destination_exists: bool,
) -> FilesResult<()> {
    if !overwrite || !destination_exists {
        dir.hard_link(temp_name, dir, destination_name)
            .map_err(map_io)?;
        return dir.remove_file(temp_name).map_err(map_io);
    }

    let backup_name = format!(".moltis-upload-backup-{}.tmp", Uuid::new_v4().simple());
    dir.rename(destination_name, dir, &backup_name)
        .map_err(map_io)?;
    if let Err(commit_error) = dir.rename(temp_name, dir, destination_name) {
        if let Err(restore_error) = dir.rename(&backup_name, dir, destination_name) {
            tracing::error!(%commit_error, %restore_error, "failed to restore managed file after overwrite failure");
        }
        return Err(map_io(commit_error));
    }
    if let Err(error) = dir.remove_file(&backup_name) {
        tracing::warn!(%error, "failed to remove managed upload backup");
    }
    Ok(())
}

impl Drop for PendingUpload {
    fn drop(&mut self) {
        drop(self.file.take());
        if !self.cleanup_armed {
            return;
        }
        match self.dir.remove_file(&self.temp_name) {
            Ok(()) => {},
            Err(error) if error.kind() == io::ErrorKind::NotFound => {},
            Err(error) => tracing::warn!(%error, "failed to clean up managed upload temp file"),
        }
    }
}
