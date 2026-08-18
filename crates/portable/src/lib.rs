//! Portable import/export of Moltis configuration, databases, and session data.
//!
//! Archives are `.tar.gz` files containing a manifest, config files, workspace
//! markdown, SQLite databases, and optionally session media and managed Files.

mod export;
mod import;
mod manifest;

pub use {
    export::{ExportOptions, export_archive},
    import::{ConflictStrategy, ImportOptions, ImportResult, ImportedItem, import_archive},
    manifest::{ArchiveInventory, ExportManifest, ManagedFilesInventory, inspect_archive},
};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod integration_tests {
    use {
        super::*,
        flate2::{Compression, read::GzDecoder, write::GzEncoder},
        std::io::Cursor,
        tar::{Archive, Builder, EntryType, Header},
    };

    static DATA_DIR_OVERRIDE_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    struct DataDirOverrideGuard;

    impl DataDirOverrideGuard {
        fn set(path: &std::path::Path) -> Self {
            moltis_config::set_data_dir(path.to_path_buf());
            Self
        }
    }

    impl Drop for DataDirOverrideGuard {
        fn drop(&mut self) {
            moltis_config::clear_data_dir();
        }
    }

    #[tokio::test]
    async fn round_trip_export_import() {
        let src_config = tempfile::tempdir().unwrap();
        let src_data = tempfile::tempdir().unwrap();

        // Create some config files.
        std::fs::write(
            src_config.path().join("moltis.toml"),
            "[server]\nport = 8080\n",
        )
        .unwrap();
        std::fs::write(
            src_config.path().join("provider_keys.json"),
            r#"{"openai":{"apiKey":"sk-test"}}"#,
        )
        .unwrap();

        // Create workspace files.
        std::fs::write(src_data.path().join("SOUL.md"), "# Soul\nBe helpful.").unwrap();
        std::fs::write(src_data.path().join("IDENTITY.md"), "name: Moltis").unwrap();

        // Create a session JSONL file.
        let sessions_dir = src_data.path().join("sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        std::fs::write(
            sessions_dir.join("main.jsonl"),
            "{\"role\":\"user\",\"content\":\"hello\"}\n",
        )
        .unwrap();

        // Export.
        let opts = ExportOptions {
            include_provider_keys: true,
            include_media: false,
            include_files: false,
        };
        let mut archive_buf = Vec::new();
        let manifest = export_archive(src_config.path(), src_data.path(), &opts, &mut archive_buf)
            .await
            .unwrap();

        assert_eq!(manifest.format_version, 1);
        assert!(
            manifest
                .inventory
                .config_files
                .contains(&"moltis.toml".to_owned())
        );
        assert!(
            manifest
                .inventory
                .config_files
                .contains(&"provider_keys.json".to_owned())
        );
        assert!(
            manifest
                .inventory
                .workspace_files
                .contains(&"SOUL.md".to_owned())
        );
        assert_eq!(manifest.inventory.session_count(), 1);

        // Inspect.
        let inspected = inspect_archive(Cursor::new(&archive_buf)).unwrap();
        assert_eq!(inspected.format_version, manifest.format_version);

        // Import into a fresh destination.
        let dst_config = tempfile::tempdir().unwrap();
        let dst_data = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dst_data.path().join("sessions")).unwrap();

        let import_opts = ImportOptions {
            conflict: ConflictStrategy::Skip,
            dry_run: false,
        };
        let result = import_archive(
            dst_config.path(),
            dst_data.path(),
            &import_opts,
            Cursor::new(&archive_buf),
        )
        .await
        .unwrap();

        // Verify files were created.
        assert!(dst_config.path().join("moltis.toml").exists());
        assert!(dst_config.path().join("provider_keys.json").exists());
        assert!(dst_data.path().join("SOUL.md").exists());
        assert!(dst_data.path().join("IDENTITY.md").exists());
        assert!(dst_data.path().join("sessions/main.jsonl").exists());

        // Verify content.
        let toml = std::fs::read_to_string(dst_config.path().join("moltis.toml")).unwrap();
        assert!(toml.contains("port = 8080"));

        let soul = std::fs::read_to_string(dst_data.path().join("SOUL.md")).unwrap();
        assert!(soul.contains("Be helpful"));

        assert!(!result.imported.is_empty());
        assert!(result.warnings.is_empty());
    }

    #[tokio::test]
    async fn import_skip_existing() {
        let src_config = tempfile::tempdir().unwrap();
        let src_data = tempfile::tempdir().unwrap();

        std::fs::write(src_config.path().join("moltis.toml"), "original").unwrap();
        std::fs::write(src_data.path().join("SOUL.md"), "original soul").unwrap();

        // Export.
        let mut buf = Vec::new();
        export_archive(
            src_config.path(),
            src_data.path(),
            &ExportOptions::default(),
            &mut buf,
        )
        .await
        .unwrap();

        // Create destination with pre-existing files.
        let dst_config = tempfile::tempdir().unwrap();
        let dst_data = tempfile::tempdir().unwrap();
        std::fs::write(dst_config.path().join("moltis.toml"), "modified").unwrap();
        std::fs::write(dst_data.path().join("SOUL.md"), "modified soul").unwrap();

        // Import with Skip strategy.
        let result = import_archive(
            dst_config.path(),
            dst_data.path(),
            &ImportOptions {
                conflict: ConflictStrategy::Skip,
                dry_run: false,
            },
            Cursor::new(&buf),
        )
        .await
        .unwrap();

        // Existing files should NOT be overwritten.
        let toml = std::fs::read_to_string(dst_config.path().join("moltis.toml")).unwrap();
        assert_eq!(toml, "modified");
        assert!(!result.skipped.is_empty());
    }

    #[tokio::test]
    async fn import_overwrite_existing() {
        let src_config = tempfile::tempdir().unwrap();
        let src_data = tempfile::tempdir().unwrap();

        std::fs::write(src_config.path().join("moltis.toml"), "from-export").unwrap();

        let mut buf = Vec::new();
        export_archive(
            src_config.path(),
            src_data.path(),
            &ExportOptions::default(),
            &mut buf,
        )
        .await
        .unwrap();

        let dst_config = tempfile::tempdir().unwrap();
        let dst_data = tempfile::tempdir().unwrap();
        std::fs::write(dst_config.path().join("moltis.toml"), "local-version").unwrap();

        let result = import_archive(
            dst_config.path(),
            dst_data.path(),
            &ImportOptions {
                conflict: ConflictStrategy::Overwrite,
                dry_run: false,
            },
            Cursor::new(&buf),
        )
        .await
        .unwrap();

        // File should be overwritten.
        let toml = std::fs::read_to_string(dst_config.path().join("moltis.toml")).unwrap();
        assert_eq!(toml, "from-export");
        assert!(result.imported.iter().any(|i| i.action == "overwritten"));
    }

    #[tokio::test]
    async fn dry_run_does_not_write() {
        let src_config = tempfile::tempdir().unwrap();
        let src_data = tempfile::tempdir().unwrap();

        std::fs::write(src_config.path().join("moltis.toml"), "content").unwrap();

        let mut buf = Vec::new();
        export_archive(
            src_config.path(),
            src_data.path(),
            &ExportOptions::default(),
            &mut buf,
        )
        .await
        .unwrap();

        let dst_config = tempfile::tempdir().unwrap();
        let dst_data = tempfile::tempdir().unwrap();

        let result = import_archive(
            dst_config.path(),
            dst_data.path(),
            &ImportOptions {
                conflict: ConflictStrategy::Skip,
                dry_run: true,
            },
            Cursor::new(&buf),
        )
        .await
        .unwrap();

        // Nothing should be written.
        assert!(!dst_config.path().join("moltis.toml").exists());
        assert!(result.imported.is_empty());
        assert!(!result.skipped.is_empty());
    }

    #[tokio::test]
    async fn managed_files_are_excluded_by_default() {
        let src_config = tempfile::tempdir().unwrap();
        let src_data = tempfile::tempdir().unwrap();
        let files_root = src_data.path().join("files");
        std::fs::create_dir_all(&files_root).unwrap();
        std::fs::write(files_root.join("private.txt"), "not exported").unwrap();

        let mut archive = Vec::new();
        let manifest = export::export_archive_from_dirs(
            src_config.path(),
            src_data.path(),
            &files_root,
            &ExportOptions::default(),
            &mut archive,
        )
        .await
        .unwrap();

        assert!(manifest.inventory.managed_files.files.is_empty());
        assert!(
            archive_paths(&archive)
                .iter()
                .all(|path| !path.contains("/files/"))
        );
    }

    #[tokio::test]
    async fn public_export_uses_explicit_data_dir_for_managed_files() {
        let _lock = DATA_DIR_OVERRIDE_TEST_LOCK.lock().await;
        let unrelated_global_data = tempfile::tempdir().unwrap();
        let _override = DataDirOverrideGuard::set(unrelated_global_data.path());
        let src_config = tempfile::tempdir().unwrap();
        let src_data = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(src_data.path().join("files")).unwrap();
        std::fs::write(src_data.path().join("files/explicit.txt"), "explicit root").unwrap();

        let mut archive = Vec::new();
        let manifest = export_archive(
            src_config.path(),
            src_data.path(),
            &ExportOptions {
                include_files: true,
                ..ExportOptions::default()
            },
            &mut archive,
        )
        .await
        .unwrap();

        assert_eq!(manifest.inventory.managed_files.files, vec!["explicit.txt"]);
        assert!(
            archive_paths(&archive)
                .iter()
                .any(|path| path.ends_with("/files/explicit.txt"))
        );
    }

    #[tokio::test]
    async fn public_import_uses_explicit_data_dir_for_managed_files() {
        let _lock = DATA_DIR_OVERRIDE_TEST_LOCK.lock().await;
        let unrelated_global_data = tempfile::tempdir().unwrap();
        let _override = DataDirOverrideGuard::set(unrelated_global_data.path());
        let src_config = tempfile::tempdir().unwrap();
        let src_data = tempfile::tempdir().unwrap();
        let src_files = src_data.path().join("files");
        std::fs::create_dir_all(&src_files).unwrap();
        std::fs::write(src_files.join("explicit.txt"), "explicit root").unwrap();
        let mut archive = Vec::new();
        export::export_archive_from_dirs(
            src_config.path(),
            src_data.path(),
            &src_files,
            &ExportOptions {
                include_files: true,
                ..ExportOptions::default()
            },
            &mut archive,
        )
        .await
        .unwrap();

        let dst_config = tempfile::tempdir().unwrap();
        let dst_data = tempfile::tempdir().unwrap();
        import_archive(
            dst_config.path(),
            dst_data.path(),
            &ImportOptions::default(),
            Cursor::new(archive),
        )
        .await
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(dst_data.path().join("files/explicit.txt")).unwrap(),
            "explicit root"
        );
        assert!(
            !unrelated_global_data
                .path()
                .join("files/explicit.txt")
                .exists()
        );
    }

    #[tokio::test]
    async fn nested_managed_files_round_trip() {
        let src_config = tempfile::tempdir().unwrap();
        let src_data = tempfile::tempdir().unwrap();
        let files_root = src_data.path().join("files");
        std::fs::create_dir_all(files_root.join("notes/empty")).unwrap();
        std::fs::write(files_root.join("notes/today.txt"), "hello files").unwrap();

        let mut archive = Vec::new();
        let manifest = export::export_archive_from_dirs(
            src_config.path(),
            src_data.path(),
            &files_root,
            &ExportOptions {
                include_files: true,
                ..ExportOptions::default()
            },
            &mut archive,
        )
        .await
        .unwrap();

        assert_eq!(manifest.inventory.managed_files.files, vec![
            "notes/today.txt"
        ]);
        assert_eq!(manifest.inventory.managed_files.total_bytes, 11);
        assert!(
            manifest
                .inventory
                .managed_files
                .directories
                .contains(&"notes/empty".to_owned())
        );

        let dst_config = tempfile::tempdir().unwrap();
        let dst_data = tempfile::tempdir().unwrap();
        let dst_files = dst_data.path().join("files");
        let result = import::import_archive_into_dirs(
            dst_config.path(),
            dst_data.path(),
            &dst_files,
            &ImportOptions::default(),
            Cursor::new(&archive),
        )
        .await
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(dst_files.join("notes/today.txt")).unwrap(),
            "hello files"
        );
        assert!(dst_files.join("notes/empty").is_dir());
        assert!(
            result
                .imported
                .iter()
                .any(|item| { item.category == "files" && item.path == "files/notes/today.txt" })
        );
    }

    #[tokio::test]
    async fn long_managed_file_path_round_trip() {
        let src_config = tempfile::tempdir().unwrap();
        let src_data = tempfile::tempdir().unwrap();
        let files_root = src_data.path().join("files");
        let long_name = format!("{}.txt", "a".repeat(140));
        std::fs::create_dir_all(&files_root).unwrap();
        std::fs::write(files_root.join(&long_name), "long path").unwrap();

        let mut archive = Vec::new();
        export::export_archive_from_dirs(
            src_config.path(),
            src_data.path(),
            &files_root,
            &ExportOptions {
                include_files: true,
                ..ExportOptions::default()
            },
            &mut archive,
        )
        .await
        .unwrap();

        let dst_config = tempfile::tempdir().unwrap();
        let dst_data = tempfile::tempdir().unwrap();
        import_archive(
            dst_config.path(),
            dst_data.path(),
            &ImportOptions::default(),
            Cursor::new(archive),
        )
        .await
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(dst_data.path().join("files").join(long_name)).unwrap(),
            "long path"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn managed_files_export_rejects_symlink() {
        use std::os::unix::fs::symlink;

        let src_config = tempfile::tempdir().unwrap();
        let src_data = tempfile::tempdir().unwrap();
        let files_root = src_data.path().join("files");
        std::fs::create_dir_all(&files_root).unwrap();
        std::fs::write(src_data.path().join("outside.txt"), "secret").unwrap();
        symlink(
            src_data.path().join("outside.txt"),
            files_root.join("link.txt"),
        )
        .unwrap();

        let result = export::export_archive_from_dirs(
            src_config.path(),
            src_data.path(),
            &files_root,
            &ExportOptions {
                include_files: true,
                ..ExportOptions::default()
            },
            Vec::new(),
        )
        .await;

        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("does not allow symlinks")
        );
    }

    #[tokio::test]
    async fn managed_files_import_rejects_traversal() {
        let archive = archive_with_raw_entry(
            "moltis-backup-test/files/../escape.txt",
            EntryType::Regular,
            b"escape",
        );
        let dst_config = tempfile::tempdir().unwrap();
        let dst_data = tempfile::tempdir().unwrap();

        let result = import::import_archive_into_dirs(
            dst_config.path(),
            dst_data.path(),
            &dst_data.path().join("files"),
            &ImportOptions::default(),
            Cursor::new(archive),
        )
        .await;

        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("invalid managed Files archive path")
        );
        assert!(!dst_data.path().join("escape.txt").exists());
    }

    #[tokio::test]
    async fn managed_files_import_rejects_special_entry() {
        let archive =
            archive_with_raw_entry("moltis-backup-test/files/link.txt", EntryType::Symlink, b"");
        let dst_config = tempfile::tempdir().unwrap();
        let dst_data = tempfile::tempdir().unwrap();

        let result = import::import_archive_into_dirs(
            dst_config.path(),
            dst_data.path(),
            &dst_data.path().join("files"),
            &ImportOptions::default(),
            Cursor::new(archive),
        )
        .await;

        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("not a regular file or directory")
        );
    }

    #[tokio::test]
    async fn manifestless_archive_has_no_file_side_effects() {
        let archive = archive_with_side_effect_entries(None);
        assert_rejected_without_file_side_effects(archive, "archive missing manifest.json").await;
    }

    #[tokio::test]
    async fn truncated_archive_has_no_file_side_effects() {
        let mut archive = archive_with_side_effect_entries(Some(test_manifest(1)));
        archive.truncate(archive.len() / 2);
        assert_rejected_without_file_side_effects(archive, "").await;
    }

    #[tokio::test]
    async fn future_manifest_has_no_file_side_effects() {
        let archive = archive_with_side_effect_entries(Some(test_manifest(2)));
        assert_rejected_without_file_side_effects(archive, "newer than supported").await;
    }

    #[tokio::test]
    async fn managed_files_manifest_mismatch_has_no_file_side_effects() {
        let archive = archive_with_side_effect_entries(Some(test_manifest(1)));
        assert_rejected_without_file_side_effects(archive, "do not match the manifest inventory")
            .await;
    }

    #[tokio::test]
    async fn managed_file_parent_conflict_has_no_file_side_effects() {
        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut builder = Builder::new(encoder);
        append_entry(
            &mut builder,
            "moltis-backup-test/files/parent",
            EntryType::Regular,
            b"parent file",
            false,
        );
        append_entry(
            &mut builder,
            "moltis-backup-test/files/parent/child.txt",
            EntryType::Regular,
            b"child",
            false,
        );
        let manifest = ExportManifest {
            inventory: ArchiveInventory {
                managed_files: ManagedFilesInventory {
                    files: vec!["parent".into(), "parent/child.txt".into()],
                    directories: Vec::new(),
                    total_bytes: 16,
                },
                ..ArchiveInventory::default()
            },
            ..test_manifest(1)
        };
        append_entry(
            &mut builder,
            "moltis-backup-test/manifest.json",
            EntryType::Regular,
            &serde_json::to_vec(&manifest).unwrap(),
            false,
        );
        let archive = builder.into_inner().unwrap().finish().unwrap();

        assert_rejected_without_file_side_effects(archive, "path crosses a file").await;
    }

    #[tokio::test]
    async fn managed_files_preview_reports_inventory_and_collisions() {
        let src_config = tempfile::tempdir().unwrap();
        let src_data = tempfile::tempdir().unwrap();
        let files_root = src_data.path().join("files");
        std::fs::create_dir_all(&files_root).unwrap();
        std::fs::write(files_root.join("existing.txt"), "from archive").unwrap();

        let mut archive = Vec::new();
        export::export_archive_from_dirs(
            src_config.path(),
            src_data.path(),
            &files_root,
            &ExportOptions {
                include_files: true,
                ..ExportOptions::default()
            },
            &mut archive,
        )
        .await
        .unwrap();

        let dst_config = tempfile::tempdir().unwrap();
        let dst_data = tempfile::tempdir().unwrap();
        let dst_files = dst_data.path().join("files");
        std::fs::create_dir_all(&dst_files).unwrap();
        std::fs::write(dst_files.join("existing.txt"), "keep me").unwrap();
        let preview = import::import_archive_into_dirs(
            dst_config.path(),
            dst_data.path(),
            &dst_files,
            &ImportOptions {
                conflict: ConflictStrategy::Skip,
                dry_run: true,
            },
            Cursor::new(&archive),
        )
        .await
        .unwrap();

        assert_eq!(preview.manifest.inventory.managed_files.files.len(), 1);
        assert_eq!(preview.manifest.inventory.managed_files.total_bytes, 12);
        assert!(preview.skipped.iter().any(|item| {
            item.category == "files"
                && item.path == "files/existing.txt"
                && item.action == "would skip (exists)"
        }));
        assert_eq!(
            std::fs::read_to_string(dst_files.join("existing.txt")).unwrap(),
            "keep me"
        );

        let overwrite_preview = import::import_archive_into_dirs(
            dst_config.path(),
            dst_data.path(),
            &dst_files,
            &ImportOptions {
                conflict: ConflictStrategy::Overwrite,
                dry_run: true,
            },
            Cursor::new(&archive),
        )
        .await
        .unwrap();
        assert!(overwrite_preview.skipped.iter().any(|item| {
            item.category == "files"
                && item.path == "files/existing.txt"
                && item.action == "would overwrite"
        }));

        let overwrite_result = import::import_archive_into_dirs(
            dst_config.path(),
            dst_data.path(),
            &dst_files,
            &ImportOptions {
                conflict: ConflictStrategy::Overwrite,
                dry_run: false,
            },
            Cursor::new(&archive),
        )
        .await
        .unwrap();
        assert!(overwrite_result.imported.iter().any(|item| {
            item.category == "files"
                && item.path == "files/existing.txt"
                && item.action == "overwritten"
        }));
        assert_eq!(
            std::fs::read_to_string(dst_files.join("existing.txt")).unwrap(),
            "from archive"
        );
    }

    fn archive_paths(bytes: &[u8]) -> Vec<String> {
        let decoder = GzDecoder::new(Cursor::new(bytes));
        let mut archive = Archive::new(decoder);
        archive
            .entries()
            .unwrap()
            .map(|entry| entry.unwrap().path().unwrap().display().to_string())
            .collect()
    }

    fn archive_with_raw_entry(path: &str, entry_type: EntryType, data: &[u8]) -> Vec<u8> {
        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut builder = Builder::new(encoder);
        let manifest = serde_json::to_vec(&ExportManifest {
            format_version: 1,
            moltis_version: "test".into(),
            created_at: "2026-05-01T00:00:00Z".into(),
            inventory: ArchiveInventory::default(),
        })
        .unwrap();
        append_entry(
            &mut builder,
            "moltis-backup-test/manifest.json",
            EntryType::Regular,
            &manifest,
            false,
        );
        append_entry(&mut builder, path, entry_type, data, true);
        builder.into_inner().unwrap().finish().unwrap()
    }

    fn archive_with_side_effect_entries(manifest: Option<ExportManifest>) -> Vec<u8> {
        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut builder = Builder::new(encoder);
        for (path, data) in [
            (
                "moltis-backup-test/config/moltis.toml",
                b"changed config".as_slice(),
            ),
            (
                "moltis-backup-test/config/new.json",
                b"new config".as_slice(),
            ),
            (
                "moltis-backup-test/workspace/SOUL.md",
                b"changed workspace".as_slice(),
            ),
            (
                "moltis-backup-test/workspace/NEW.md",
                b"new workspace".as_slice(),
            ),
            (
                "moltis-backup-test/files/existing.txt",
                b"changed file".as_slice(),
            ),
            ("moltis-backup-test/files/new.txt", b"new file".as_slice()),
        ] {
            append_entry(&mut builder, path, EntryType::Regular, data, false);
        }
        if let Some(manifest) = manifest {
            let data = serde_json::to_vec(&manifest).unwrap();
            append_entry(
                &mut builder,
                "moltis-backup-test/manifest.json",
                EntryType::Regular,
                &data,
                false,
            );
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    fn test_manifest(format_version: u32) -> ExportManifest {
        ExportManifest {
            format_version,
            moltis_version: "test".into(),
            created_at: "2026-05-01T00:00:00Z".into(),
            inventory: ArchiveInventory::default(),
        }
    }

    async fn assert_rejected_without_file_side_effects(archive: Vec<u8>, expected_error: &str) {
        let config_dir = tempfile::tempdir().unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        let files_dir = data_dir.path().join("files");
        std::fs::create_dir_all(&files_dir).unwrap();
        std::fs::write(config_dir.path().join("moltis.toml"), "original config").unwrap();
        std::fs::write(data_dir.path().join("SOUL.md"), "original workspace").unwrap();
        std::fs::write(files_dir.join("existing.txt"), "original file").unwrap();

        let error = import_archive(
            config_dir.path(),
            data_dir.path(),
            &ImportOptions {
                conflict: ConflictStrategy::Overwrite,
                dry_run: false,
            },
            Cursor::new(archive),
        )
        .await
        .unwrap_err();

        if !expected_error.is_empty() {
            assert!(error.to_string().contains(expected_error), "{error:#}");
        }
        assert_eq!(
            std::fs::read_to_string(config_dir.path().join("moltis.toml")).unwrap(),
            "original config"
        );
        assert_eq!(
            std::fs::read_to_string(data_dir.path().join("SOUL.md")).unwrap(),
            "original workspace"
        );
        assert_eq!(
            std::fs::read_to_string(files_dir.join("existing.txt")).unwrap(),
            "original file"
        );
        assert!(!config_dir.path().join("new.json").exists());
        assert!(!data_dir.path().join("NEW.md").exists());
        assert!(!files_dir.join("new.txt").exists());
        assert!(std::fs::read_dir(files_dir).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".moltis-upload-")
        }));
    }

    fn append_entry<W: std::io::Write>(
        builder: &mut Builder<W>,
        path: &str,
        entry_type: EntryType,
        data: &[u8],
        raw_path: bool,
    ) {
        let mut header = Header::new_gnu();
        header.set_entry_type(entry_type);
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        if raw_path {
            let path_bytes = path.as_bytes();
            header.as_mut_bytes()[..path_bytes.len()].copy_from_slice(path_bytes);
        } else {
            header.set_path(path).unwrap();
        }
        header.set_cksum();
        builder.append(&header, data).unwrap();
    }
}
