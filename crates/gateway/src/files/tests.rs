use {super::*, std::io::Read, tokio::io::AsyncWriteExt};

fn service() -> (tempfile::TempDir, LocalFilesService) {
    let temp = tempfile::tempdir().unwrap();
    let service = LocalFilesService::new(temp.path()).unwrap();
    (temp, service)
}

async fn upload(service: &LocalFilesService, path: &str, bytes: &[u8], overwrite: bool) {
    let mut pending = service.begin_upload(path, overwrite).unwrap();
    pending.writer().unwrap().write_all(bytes).await.unwrap();
    pending.commit().await.unwrap();
}

#[tokio::test]
async fn round_trip_list_move_download_and_delete() {
    let (_temp, service) = service();
    service.create_directory("notes").unwrap();
    upload(&service, "notes/hello.txt", b"hello", false).await;

    let listing = service.list("notes").unwrap();
    assert_eq!(listing.path, "notes");
    assert_eq!(listing.entries.len(), 1);
    assert_eq!(listing.entries[0].path, "notes/hello.txt");
    assert_eq!(listing.entries[0].metadata.kind, EntryKind::File);
    assert_eq!(listing.entries[0].metadata.size_bytes, Some(5));

    service
        .move_entry("notes/hello.txt", "hello.txt", false)
        .unwrap();
    let mut opened = service.open_download("hello.txt").unwrap();
    let mut contents = String::new();
    opened.file.read_to_string(&mut contents).unwrap();
    assert_eq!(contents, "hello");
    service.delete("hello.txt", false).unwrap();
    service.delete("notes", false).unwrap();
    assert!(service.list("").unwrap().entries.is_empty());
}

#[test]
fn rejects_unsafe_and_excessive_paths() {
    let (_temp, service) = service();
    for path in [
        "/absolute",
        "../escape",
        "a/../escape",
        "a/./file",
        "a\\file",
        "a\0file",
        "a\nfile",
        "a//file",
        "a/",
        "C:/prefixed",
    ] {
        assert!(matches!(
            service.create_directory(path),
            Err(FilesError::InvalidPath)
        ));
    }
    let deep = std::iter::repeat_n("a", 65).collect::<Vec<_>>().join("/");
    assert!(matches!(service.list(&deep), Err(FilesError::InvalidPath)));
    assert!(matches!(
        service.create_directory(&"a".repeat(256)),
        Err(FilesError::InvalidPath)
    ));
    assert!(matches!(
        service.list(&"a".repeat(4097)),
        Err(FilesError::InvalidPath)
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn rejects_symlink_ancestors_and_final_symlinks() {
    use std::os::unix::fs::symlink;

    let (temp, service) = service();
    std::fs::create_dir(temp.path().join("real")).unwrap();
    std::fs::write(temp.path().join("real/file"), b"secret").unwrap();
    symlink("real", temp.path().join("linked-dir")).unwrap();
    symlink("real/file", temp.path().join("linked-file")).unwrap();

    assert!(matches!(
        service.list("linked-dir"),
        Err(FilesError::UnsupportedEntry)
    ));
    assert!(matches!(
        service.open_download("linked-file"),
        Err(FilesError::UnsupportedEntry)
    ));
    service.delete("linked-file", false).unwrap();
}

#[cfg(unix)]
#[test]
fn rejects_special_files() {
    use std::os::unix::net::UnixListener;

    let (temp, service) = service();
    let _listener = UnixListener::bind(temp.path().join("socket")).unwrap();
    assert!(matches!(
        service.open_download("socket"),
        Err(FilesError::UnsupportedEntry)
    ));
    let listing = service.list("").unwrap();
    assert_eq!(listing.entries[0].metadata.kind, EntryKind::Other);
}

#[cfg(unix)]
#[test]
fn rejects_a_symlinked_managed_root() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("target");
    std::fs::create_dir(&target).unwrap();
    let linked_root = temp.path().join("files");
    symlink(&target, &linked_root).unwrap();

    assert!(matches!(
        LocalFilesService::new(&linked_root),
        Err(FilesError::UnsupportedEntry)
    ));
}

#[cfg(unix)]
#[test]
fn recursive_delete_does_not_follow_child_symlinks() {
    use std::os::unix::fs::symlink;

    let (temp, service) = service();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("keep"), b"keep").unwrap();
    service.create_directory("tree").unwrap();
    symlink(outside.path(), temp.path().join("tree/outside")).unwrap();

    service.delete("tree", true).unwrap();
    assert_eq!(std::fs::read(outside.path().join("keep")).unwrap(), b"keep");
}

#[tokio::test]
async fn conflicts_and_overwrite_only_replace_regular_files() {
    let (_temp, service) = service();
    upload(&service, "file", b"old", false).await;
    assert!(matches!(
        service.begin_upload("file", false),
        Err(FilesError::Conflict)
    ));
    upload(&service, "file", b"new", true).await;
    service.create_directory("dir").unwrap();
    assert!(matches!(
        service.begin_upload("dir", true),
        Err(FilesError::UnsupportedEntry)
    ));
    assert!(matches!(
        service.move_entry("dir", "file", true),
        Err(FilesError::UnsupportedEntry)
    ));
}

#[tokio::test]
async fn concurrent_no_overwrite_commits_do_not_replace_the_winner() {
    let (_temp, service) = service();
    let mut first = service.begin_upload("race.txt", false).unwrap();
    let mut second = service.begin_upload("race.txt", false).unwrap();
    first.writer().unwrap().write_all(b"first").await.unwrap();
    second.writer().unwrap().write_all(b"second").await.unwrap();

    first.commit().await.unwrap();
    assert!(matches!(second.commit().await, Err(FilesError::Conflict)));

    let mut opened = service.open_download("race.txt").unwrap();
    let mut contents = String::new();
    opened.file.read_to_string(&mut contents).unwrap();
    assert_eq!(contents, "first");
}

#[tokio::test]
async fn recursive_delete_protects_root_and_temp_files_are_cleaned() {
    let (temp, service) = service();
    service.create_directory("tree").unwrap();
    service.create_directory("tree/child").unwrap();
    upload(&service, "tree/child/file", b"data", false).await;
    assert!(service.delete("tree", false).is_err());
    service.delete("tree", true).unwrap();
    assert!(matches!(
        service.delete("", true),
        Err(FilesError::RootMutation)
    ));

    let pending = service.begin_upload("abandoned", false).unwrap();
    assert!(service.list("").unwrap().entries.is_empty());
    drop(pending);
    assert!(
        std::fs::read_dir(temp.path())
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .is_empty()
    );
}
