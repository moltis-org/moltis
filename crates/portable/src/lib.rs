//! Portable import/export of Moltis configuration, databases, and session data.
//!
//! Archives are `.tar.gz` files containing a manifest, config files, workspace
//! markdown, SQLite databases, and optionally session media.

mod export;
mod import;
mod manifest;

pub use {
    export::{ExportOptions, export_archive},
    import::{ConflictStrategy, ImportOptions, ImportResult, ImportedItem, import_archive},
    manifest::{ArchiveInventory, ExportManifest, inspect_archive},
};

#[cfg(test)]
mod integration_tests {
    use {super::*, std::io::Cursor};

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
}
