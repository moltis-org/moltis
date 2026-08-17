use {
    cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt},
    cap_std::fs::{Dir, OpenOptions},
    std::{io, path::Path, sync::Arc},
    tokio::sync::{OwnedSemaphorePermit, Semaphore},
    uuid::Uuid,
};

use super::{
    DirectoryListing, FileEntry, FilesError, FilesResult, OpenedFile, PendingUpload,
    error::map_io,
    path::{LogicalPath, TEMP_NAME_PREFIX},
    types::entry_metadata,
};

pub struct LocalFilesService {
    root: Dir,
    upload_slots: Arc<Semaphore>,
}

impl LocalFilesService {
    pub fn new(root: impl AsRef<Path>) -> FilesResult<Self> {
        let root = root.as_ref();
        let parent_path = root.parent().ok_or(FilesError::InvalidPath)?;
        let root_name = root.file_name().ok_or(FilesError::InvalidPath)?;
        std::fs::create_dir_all(parent_path).map_err(map_io)?;
        let parent =
            Dir::open_ambient_dir(parent_path, cap_std::ambient_authority()).map_err(map_io)?;
        let root_metadata = match parent.symlink_metadata(root_name) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                parent.create_dir(root_name).map_err(map_io)?;
                parent.symlink_metadata(root_name).map_err(map_io)?
            },
            Err(error) => return Err(map_io(error)),
        };
        if root_metadata.is_symlink() {
            return Err(FilesError::UnsupportedEntry);
        }
        if !root_metadata.is_dir() {
            return Err(FilesError::NotDirectory);
        }
        let root = parent.open_dir_nofollow(root_name).map_err(map_io)?;
        let metadata = root.dir_metadata().map_err(map_io)?;
        if !metadata.is_dir() {
            return Err(FilesError::NotDirectory);
        }
        Ok(Self {
            root,
            upload_slots: Arc::new(Semaphore::new(4)),
        })
    }

    pub async fn acquire_upload_slot(&self) -> FilesResult<OwnedSemaphorePermit> {
        Arc::clone(&self.upload_slots)
            .acquire_owned()
            .await
            .map_err(|error| FilesError::Io(io::Error::other(error)))
    }

    pub fn list(&self, path: &str) -> FilesResult<DirectoryListing> {
        let logical = LogicalPath::parse(path, true)?;
        let dir = self.open_directory(logical.components())?;
        let mut entries = Vec::new();
        for entry in dir.entries().map_err(map_io)? {
            let entry = entry.map_err(map_io)?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| FilesError::UnsupportedEntry)?;
            if name.starts_with(TEMP_NAME_PREFIX) {
                continue;
            }
            let metadata = dir.symlink_metadata(&name).map_err(map_io)?;
            let child_path = if logical.is_root() {
                name.clone()
            } else {
                format!("{}/{name}", logical.raw())
            };
            let child_path = LogicalPath::parse(&child_path, false)?;
            entries.push(FileEntry {
                name,
                path: child_path.raw().to_owned(),
                metadata: entry_metadata(&metadata)?,
            });
        }
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(DirectoryListing {
            path: logical.raw().to_owned(),
            entries,
        })
    }

    pub fn create_directory(&self, path: &str) -> FilesResult<FileEntry> {
        let logical = LogicalPath::parse(path, false)?;
        let parent = self.open_directory(logical.parent_components())?;
        let name = logical.file_name()?;
        parent.create_dir(name).map_err(map_io)?;
        let metadata = parent.symlink_metadata(name).map_err(map_io)?;
        Ok(FileEntry {
            name: name.to_owned(),
            path: logical.raw().to_owned(),
            metadata: entry_metadata(&metadata)?,
        })
    }

    pub fn begin_upload(&self, path: &str, overwrite: bool) -> FilesResult<PendingUpload> {
        let logical = LogicalPath::parse(path, false)?;
        let parent = self.open_directory(logical.parent_components())?;
        let name = logical.file_name()?;
        match parent.symlink_metadata(name) {
            Ok(_) if !overwrite => return Err(FilesError::Conflict),
            Ok(metadata) if !metadata.is_file() => return Err(FilesError::UnsupportedEntry),
            Ok(_) => {},
            Err(error) if error.kind() == io::ErrorKind::NotFound => {},
            Err(error) => return Err(map_io(error)),
        }

        let temp_name = format!("{TEMP_NAME_PREFIX}{}.tmp", Uuid::new_v4().simple());
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No);
        let file = parent
            .open_with(&temp_name, &options)
            .map_err(map_io)?
            .into_std();
        Ok(PendingUpload::new(
            parent,
            temp_name,
            name.to_owned(),
            logical.raw().to_owned(),
            tokio::fs::File::from_std(file),
            overwrite,
        ))
    }

    pub fn open_download(&self, path: &str) -> FilesResult<OpenedFile> {
        let logical = LogicalPath::parse(path, false)?;
        let parent = self.open_directory(logical.parent_components())?;
        let name = logical.file_name()?;
        let metadata = parent.symlink_metadata(name).map_err(map_io)?;
        if metadata.is_symlink() || (!metadata.is_file() && !metadata.is_dir()) {
            return Err(FilesError::UnsupportedEntry);
        }
        if !metadata.is_file() {
            return Err(FilesError::NotFile);
        }
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let file = parent.open_with(name, &options).map_err(map_io)?;
        let opened_metadata = file.metadata().map_err(map_io)?;
        if !opened_metadata.is_file() {
            return Err(FilesError::NotFile);
        }
        Ok(OpenedFile {
            file: file.into_std(),
            name: name.to_owned(),
            metadata: entry_metadata(&opened_metadata)?,
        })
    }

    pub fn move_entry(
        &self,
        source: &str,
        destination: &str,
        overwrite: bool,
    ) -> FilesResult<FileEntry> {
        let source = LogicalPath::parse(source, false)?;
        let destination = LogicalPath::parse(destination, false)?;
        if source == destination {
            return Err(FilesError::Conflict);
        }
        if destination.components().starts_with(source.components()) {
            return Err(FilesError::InvalidPath);
        }

        let source_parent = self.open_directory(source.parent_components())?;
        let source_name = source.file_name()?;
        let source_metadata = source_parent
            .symlink_metadata(source_name)
            .map_err(map_io)?;
        if source_metadata.is_symlink() || (!source_metadata.is_file() && !source_metadata.is_dir())
        {
            return Err(FilesError::UnsupportedEntry);
        }

        let destination_parent = self.open_directory(destination.parent_components())?;
        let destination_name = destination.file_name()?;
        let destination_exists = match destination_parent.symlink_metadata(destination_name) {
            Ok(_) if !overwrite => return Err(FilesError::Conflict),
            Ok(metadata)
                if !source_metadata.is_file() || !metadata.is_file() || metadata.is_symlink() =>
            {
                return Err(FilesError::UnsupportedEntry);
            },
            Ok(_) => true,
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => return Err(map_io(error)),
        };

        if !overwrite && source_metadata.is_file() {
            source_parent
                .hard_link(source_name, &destination_parent, destination_name)
                .map_err(map_io)?;
            source_parent.remove_file(source_name).map_err(map_io)?;
        } else {
            move_entry_between_dirs(
                &source_parent,
                source_name,
                &destination_parent,
                destination_name,
                overwrite,
                destination_exists,
            )?;
        }
        let metadata = destination_parent
            .symlink_metadata(destination_name)
            .map_err(map_io)?;
        Ok(FileEntry {
            name: destination_name.to_owned(),
            path: destination.raw().to_owned(),
            metadata: entry_metadata(&metadata)?,
        })
    }

    pub fn delete(&self, path: &str, recursive: bool) -> FilesResult<()> {
        let logical = LogicalPath::parse(path, false)?;
        let parent = self.open_directory(logical.parent_components())?;
        let name = logical.file_name()?;
        let metadata = parent.symlink_metadata(name).map_err(map_io)?;
        if metadata.is_symlink() || (!metadata.is_file() && !metadata.is_dir()) {
            return parent.remove_file(name).map_err(map_io);
        }
        if metadata.is_file() {
            return parent.remove_file(name).map_err(map_io);
        }
        if !metadata.is_dir() {
            return Err(FilesError::UnsupportedEntry);
        }
        let dir = parent.open_dir_nofollow(name).map_err(map_io)?;
        if recursive {
            dir.remove_open_dir_all().map_err(map_io)
        } else {
            dir.remove_open_dir().map_err(map_io)
        }
    }

    fn open_directory(&self, components: &[String]) -> FilesResult<Dir> {
        components
            .iter()
            .try_fold(self.root.try_clone().map_err(map_io)?, |dir, component| {
                let metadata = dir.symlink_metadata(component).map_err(map_io)?;
                if metadata.is_symlink() {
                    return Err(FilesError::UnsupportedEntry);
                }
                if !metadata.is_dir() {
                    return Err(FilesError::NotDirectory);
                }
                dir.open_dir_nofollow(component).map_err(map_io)
            })
    }
}

#[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
fn move_entry_between_dirs(
    source_parent: &Dir,
    source_name: &str,
    destination_parent: &Dir,
    destination_name: &str,
    overwrite: bool,
    _destination_exists: bool,
) -> FilesResult<()> {
    if !overwrite {
        return rustix::fs::renameat_with(
            source_parent,
            source_name,
            destination_parent,
            destination_name,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(io::Error::from)
        .map_err(map_io);
    }
    source_parent
        .rename(source_name, destination_parent, destination_name)
        .map_err(map_io)
}

#[cfg(all(
    not(windows),
    not(any(target_os = "linux", target_os = "android", target_vendor = "apple"))
))]
fn move_entry_between_dirs(
    source_parent: &Dir,
    source_name: &str,
    destination_parent: &Dir,
    destination_name: &str,
    _overwrite: bool,
    _destination_exists: bool,
) -> FilesResult<()> {
    source_parent
        .rename(source_name, destination_parent, destination_name)
        .map_err(map_io)
}

#[cfg(windows)]
fn move_entry_between_dirs(
    source_parent: &Dir,
    source_name: &str,
    destination_parent: &Dir,
    destination_name: &str,
    overwrite: bool,
    destination_exists: bool,
) -> FilesResult<()> {
    if !overwrite || !destination_exists {
        return source_parent
            .rename(source_name, destination_parent, destination_name)
            .map_err(map_io);
    }

    let backup_name = format!(".moltis-upload-backup-{}.tmp", Uuid::new_v4().simple());
    destination_parent
        .rename(destination_name, destination_parent, &backup_name)
        .map_err(map_io)?;
    if let Err(move_error) = source_parent.rename(source_name, destination_parent, destination_name)
    {
        if let Err(restore_error) =
            destination_parent.rename(&backup_name, destination_parent, destination_name)
        {
            tracing::error!(%move_error, %restore_error, "failed to restore managed file after move failure");
        }
        return Err(map_io(move_error));
    }
    if let Err(error) = destination_parent.remove_file(&backup_name) {
        tracing::warn!(%error, "failed to remove managed file move backup");
    }
    Ok(())
}
