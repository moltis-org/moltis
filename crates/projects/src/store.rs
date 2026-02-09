use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use {anyhow::Result, async_trait::async_trait};

use crate::types::Project;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Trait for persisting projects. Implementations can be TOML-file-backed,
/// SQLite, etc.
#[async_trait]
pub trait ProjectStore: Send + Sync {
    async fn list(&self) -> Result<Vec<Project>>;
    async fn get(&self, id: &str) -> Result<Option<Project>>;
    async fn upsert(&self, project: Project) -> Result<()>;
    async fn delete(&self, id: &str) -> Result<()>;
}

// ── TOML file-backed implementation ──────────────────────────────────

#[derive(Debug, serde::Serialize, serde::Deserialize, Default)]
struct TomlFile {
    #[serde(default)]
    projects: Vec<Project>,
}

/// Stores projects in a TOML file at the given path.
pub struct TomlProjectStore {
    path: PathBuf,
}

impl TomlProjectStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn read_file(&self) -> Result<TomlFile> {
        if self.path.exists() {
            let data = fs::read_to_string(&self.path)?;
            Ok(toml::from_str(&data)?)
        } else {
            Ok(TomlFile::default())
        }
    }

    fn write_file(&self, file: &TomlFile) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = toml::to_string_pretty(file)?;
        fs::write(&self.path, data)?;
        Ok(())
    }
}

#[async_trait]
impl ProjectStore for TomlProjectStore {
    async fn list(&self) -> Result<Vec<Project>> {
        let mut projects = self.read_file()?.projects;
        projects.sort_by_key(|p| std::cmp::Reverse(p.updated_at));
        Ok(projects)
    }

    async fn get(&self, id: &str) -> Result<Option<Project>> {
        Ok(self.read_file()?.projects.into_iter().find(|p| p.id == id))
    }

    async fn upsert(&self, project: Project) -> Result<()> {
        let mut file = self.read_file()?;
        if let Some(existing) = file.projects.iter_mut().find(|p| p.id == project.id) {
            *existing = project;
        } else {
            file.projects.push(project);
        }
        self.write_file(&file)
    }

    async fn delete(&self, id: &str) -> Result<()> {
        let mut file = self.read_file()?;
        file.projects.retain(|p| p.id != id);
        self.write_file(&file)
    }
}

// ── SQLite-backed implementation ────────────────────────────────────

/// Stores projects in a SQLite database.
pub struct SqliteProjectStore {
    pool: sqlx::SqlitePool,
}

impl SqliteProjectStore {
    pub fn new(pool: sqlx::SqlitePool) -> Self {
        Self { pool }
    }

    /// Initialize the projects table schema.
    ///
    /// **Deprecated**: Schema is now managed by sqlx migrations in the gateway crate.
    /// This method is retained for tests that use in-memory databases.
    #[doc(hidden)]
    pub async fn init(pool: &sqlx::SqlitePool) -> Result<()> {
        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS projects (
                id               TEXT    PRIMARY KEY,
                label            TEXT    NOT NULL,
                directory        TEXT    NOT NULL,
                system_prompt    TEXT,
                auto_worktree    INTEGER NOT NULL DEFAULT 0,
                setup_command    TEXT,
                teardown_command TEXT,
                branch_prefix    TEXT,
                sandbox_image    TEXT,
                detected         INTEGER NOT NULL DEFAULT 0,
                created_at       INTEGER NOT NULL,
                updated_at       INTEGER NOT NULL
            )"#,
        )
        .execute(pool)
        .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_projects_updated_at ON projects(updated_at)")
            .execute(pool)
            .await
            .ok();

        Ok(())
    }
}

#[async_trait]
impl ProjectStore for SqliteProjectStore {
    async fn list(&self) -> Result<Vec<Project>> {
        let rows =
            sqlx::query_as::<_, ProjectRow>("SELECT * FROM projects ORDER BY updated_at DESC")
                .fetch_all(&self.pool)
                .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn get(&self, id: &str) -> Result<Option<Project>> {
        let row = sqlx::query_as::<_, ProjectRow>("SELECT * FROM projects WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(Into::into))
    }

    async fn upsert(&self, project: Project) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO projects (id, label, directory, system_prompt, auto_worktree, setup_command, teardown_command, branch_prefix, sandbox_image, detected, created_at, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(id) DO UPDATE SET
                 label = excluded.label,
                 directory = excluded.directory,
                 system_prompt = excluded.system_prompt,
                 auto_worktree = excluded.auto_worktree,
                 setup_command = excluded.setup_command,
                 teardown_command = excluded.teardown_command,
                 branch_prefix = excluded.branch_prefix,
                 sandbox_image = excluded.sandbox_image,
                 detected = excluded.detected,
                 updated_at = excluded.updated_at"#,
        )
        .bind(&project.id)
        .bind(&project.label)
        .bind(project.directory.to_string_lossy().as_ref())
        .bind(&project.system_prompt)
        .bind(project.auto_worktree as i32)
        .bind(&project.setup_command)
        .bind(&project.teardown_command)
        .bind(&project.branch_prefix)
        .bind(&project.sandbox_image)
        .bind(project.detected as i32)
        .bind(project.created_at as i64)
        .bind(project.updated_at as i64)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn delete(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM projects WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

/// Internal row type for sqlx mapping.
#[derive(sqlx::FromRow)]
struct ProjectRow {
    id: String,
    label: String,
    directory: String,
    system_prompt: Option<String>,
    auto_worktree: i32,
    setup_command: Option<String>,
    teardown_command: Option<String>,
    branch_prefix: Option<String>,
    sandbox_image: Option<String>,
    detected: i32,
    created_at: i64,
    updated_at: i64,
}

impl From<ProjectRow> for Project {
    fn from(r: ProjectRow) -> Self {
        Self {
            id: r.id,
            label: r.label,
            directory: PathBuf::from(r.directory),
            system_prompt: r.system_prompt,
            auto_worktree: r.auto_worktree != 0,
            setup_command: r.setup_command,
            teardown_command: r.teardown_command,
            branch_prefix: r.branch_prefix,
            sandbox_image: r.sandbox_image,
            detected: r.detected != 0,
            created_at: r.created_at as u64,
            updated_at: r.updated_at as u64,
        }
    }
}

/// Create a new project with auto-derived fields.
pub fn new_project(id: String, label: String, directory: PathBuf) -> Project {
    let now = now_ms();
    Project {
        id,
        label,
        directory,
        system_prompt: None,
        auto_worktree: false,
        setup_command: None,
        teardown_command: None,
        branch_prefix: None,
        sandbox_image: None,
        detected: false,
        created_at: now,
        updated_at: now,
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_toml_store_crud() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("projects.toml");
        let store = TomlProjectStore::new(path);

        // Empty initially
        assert!(store.list().await.unwrap().is_empty());

        // Upsert
        let p = new_project("test".into(), "Test".into(), "/tmp/test".into());
        store.upsert(p).await.unwrap();
        assert_eq!(store.list().await.unwrap().len(), 1);

        // Get
        let found = store.get("test").await.unwrap().unwrap();
        assert_eq!(found.label, "Test");

        // Update
        let mut updated = found;
        updated.label = "Updated".into();
        store.upsert(updated).await.unwrap();
        assert_eq!(store.list().await.unwrap().len(), 1);
        assert_eq!(store.get("test").await.unwrap().unwrap().label, "Updated");

        // Delete
        store.delete("test").await.unwrap();
        assert!(store.list().await.unwrap().is_empty());
    }

    async fn sqlite_pool() -> sqlx::SqlitePool {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        SqliteProjectStore::init(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn test_sqlite_store_crud() {
        let pool = sqlite_pool().await;
        let store = SqliteProjectStore::new(pool);

        assert!(store.list().await.unwrap().is_empty());

        let p = new_project("test".into(), "Test".into(), "/tmp/test".into());
        store.upsert(p).await.unwrap();
        assert_eq!(store.list().await.unwrap().len(), 1);

        let found = store.get("test").await.unwrap().unwrap();
        assert_eq!(found.label, "Test");

        let mut updated = found;
        updated.label = "Updated".into();
        store.upsert(updated).await.unwrap();
        assert_eq!(store.list().await.unwrap().len(), 1);
        assert_eq!(store.get("test").await.unwrap().unwrap().label, "Updated");

        store.delete("test").await.unwrap();
        assert!(store.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_toml_store_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("projects.toml");

        {
            let store = TomlProjectStore::new(path.clone());
            store
                .upsert(new_project("a".into(), "A".into(), "/a".into()))
                .await
                .unwrap();
        }

        // New store instance reads from disk
        let store = TomlProjectStore::new(path);
        let list = store.list().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "a");
    }
}
