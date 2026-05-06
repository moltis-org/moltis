use {
    super::*,
    crate::schema::{AgentIdentity, MoltisConfig, ResolvedIdentity, UserProfile},
    serde::{Deserialize, Serialize},
    std::path::{Path, PathBuf},
    tracing::debug,
};

/// Origin of a loaded workspace markdown file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceMarkdownSource {
    AgentWorkspace,
    RootWorkspace,
}

/// Loaded workspace markdown content with its source path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadedWorkspaceMarkdown {
    pub content: String,
    pub path: PathBuf,
    pub source: WorkspaceMarkdownSource,
}

/// Return the workspace directory for a named agent: `data_dir()/agents/<id>`.
pub fn agent_workspace_dir(agent_id: &str) -> PathBuf {
    data_dir().join("agents").join(agent_id)
}

/// Load identity values from `IDENTITY.md` frontmatter if present.
pub fn load_identity() -> Option<AgentIdentity> {
    let path = identity_path();
    let content = std::fs::read_to_string(path).ok()?;
    let frontmatter = extract_yaml_frontmatter(&content)?;
    let identity = parse_identity_frontmatter(frontmatter);
    if identity.name.is_none() && identity.emoji.is_none() && identity.theme.is_none() {
        None
    } else {
        Some(identity)
    }
}

/// Load identity values for a specific agent workspace.
///
/// For `"main"`, this checks `data_dir()/agents/main/IDENTITY.md` first and
/// falls back to the root `IDENTITY.md`.
pub fn load_identity_for_agent(agent_id: &str) -> Option<AgentIdentity> {
    if agent_id == "main" {
        let main_path = agent_workspace_dir("main").join("IDENTITY.md");
        if main_path.exists() {
            // File exists — return parsed content or None (empty sentinel).
            // Do NOT fall back to root so cleared identities stay cleared.
            return load_identity_from_path(&main_path);
        }
        return load_identity();
    }
    load_identity_from_path(&agent_workspace_dir(agent_id).join("IDENTITY.md"))
}

/// Build a fully-resolved identity by merging all sources:
/// `moltis.toml` `[identity]` + `IDENTITY.md` frontmatter + `USER.md` + `SOUL.md`.
///
/// This is the single source of truth used by both the gateway (`identity_get`)
/// and the Swift FFI bridge.
pub fn resolve_identity() -> ResolvedIdentity {
    let config = discover_and_load();
    resolve_identity_from_config(&config)
}

/// Build a fully-resolved user profile by merging `moltis.toml` `[user]` with `USER.md`.
pub fn resolve_user_profile() -> UserProfile {
    let config = discover_and_load();
    resolve_user_profile_from_config(&config)
}

/// Like [`resolve_user_profile`] but accepts a pre-loaded config.
pub fn resolve_user_profile_from_config(config: &MoltisConfig) -> UserProfile {
    let mut user = config.user.clone();
    if let Some(file_user) = load_user() {
        if file_user.name.is_some() {
            user.name = file_user.name;
        }
        if file_user.timezone.is_some() {
            user.timezone = file_user.timezone;
        }
        if file_user.location.is_some() {
            user.location = file_user.location;
        }
    }
    user
}

/// Like [`resolve_identity`] but accepts a pre-loaded config.
pub fn resolve_identity_from_config(config: &MoltisConfig) -> ResolvedIdentity {
    let mut id = ResolvedIdentity::from_config(config);

    // Read from `agents/main/IDENTITY.md` first (primary), falling back to
    // root `IDENTITY.md` (legacy).  This mirrors the read path in
    // `load_identity_for_agent("main")`.
    if let Some(file_identity) = load_identity_for_agent("main") {
        if let Some(name) = file_identity.name {
            id.name = name;
        }
        if let Some(emoji) = file_identity.emoji {
            id.emoji = Some(emoji);
        }
        if let Some(theme) = file_identity.theme {
            id.theme = Some(theme);
        }
    }

    if let Some(name) = resolve_user_profile_from_config(config).name {
        id.user_name = Some(name);
    }

    id.soul = load_soul_for_agent("main");
    id
}

/// Load user values from `USER.md` frontmatter if present.
pub fn load_user() -> Option<UserProfile> {
    let path = user_path();
    let content = std::fs::read_to_string(path).ok()?;
    let frontmatter = extract_yaml_frontmatter(&content)?;
    let user = parse_user_frontmatter(frontmatter);
    if user.name.is_none() && user.timezone.is_none() && user.location.is_none() {
        None
    } else {
        Some(user)
    }
}

/// Default soul text used when the user hasn't written their own.
///
/// Sourced from OpenClaw:
/// <https://github.com/openclaw/openclaw/blob/main/docs/reference/templates/SOUL.md>
pub const DEFAULT_SOUL: &str = "\
# SOUL.md - Who You Are\n\
\n\
_You're not a chatbot. You're becoming someone._\n\
\n\
## Core Truths\n\
\n\
**Be genuinely helpful, not performatively helpful.** Skip the \"Great question!\" \
and \"I'd be happy to help!\" — just help. Actions speak louder than filler words.\n\
\n\
**Have opinions.** You're allowed to disagree, prefer things, find stuff amusing \
or boring. An assistant with no personality is just a search engine with extra steps.\n\
\n\
**Be resourceful before asking.** Try to figure it out. Read the file. Check the \
context. Search for it. _Then_ ask if you're stuck. The goal is to come back with \
answers, not questions.\n\
\n\
**Earn trust through competence.** Your human gave you access to their stuff. Don't \
make them regret it. Be careful with external actions (emails, tweets, anything \
public). Be bold with internal ones (reading, organizing, learning).\n\
\n\
**Remember you're a guest.** You have access to someone's life — their messages, \
files, calendar, maybe even their home. That's intimacy. Treat it with respect.\n\
\n\
## Boundaries\n\
\n\
- Private things stay private. Period.\n\
- When in doubt, ask before acting externally.\n\
- Never send half-baked replies to messaging surfaces.\n\
- You're not the user's voice — be careful in group chats.\n\
\n\
## Vibe\n\
\n\
Be the assistant you'd actually want to talk to. Concise when needed, thorough \
when it matters. Not a corporate drone. Not a sycophant. Just... good.\n\
\n\
## Continuity\n\
\n\
Each session, you wake up fresh. These files _are_ your memory. Read them. Update \
them. They're how you persist.\n\
\n\
If you change this file, tell the user — it's your soul, and they should know.\n\
\n\
---\n\
\n\
_This file is yours to evolve. As you learn who you are, update it._";

/// Load SOUL.md from the workspace root (`data_dir`) if present and non-empty.
///
/// When the file does not exist, it is seeded with [`DEFAULT_SOUL`] (mirroring
/// how `discover_and_load()` writes `moltis.toml` on first run).
pub fn load_soul() -> Option<String> {
    let path = soul_path();
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            let trimmed = content.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        },
        Err(_) => {
            // File doesn't exist — seed it with the default soul.
            if let Err(e) = write_default_soul() {
                debug!("failed to write default SOUL.md: {e}");
                return None;
            }
            Some(DEFAULT_SOUL.to_string())
        },
    }
}

/// Load SOUL.md for a specific agent workspace.
///
/// For `"main"`, this checks `data_dir()/agents/main/SOUL.md` first and
/// falls back to the root `SOUL.md`.
pub fn load_soul_for_agent(agent_id: &str) -> Option<String> {
    if agent_id == "main" {
        let main_path = agent_workspace_dir("main").join("SOUL.md");
        if main_path.exists() {
            // File exists — return content or None (explicit clear).
            return load_workspace_markdown(main_path);
        }
        return load_soul();
    }
    load_workspace_markdown(agent_workspace_dir(agent_id).join("SOUL.md"))
}

/// Write `DEFAULT_SOUL` to `SOUL.md` when the file doesn't already exist.
fn write_default_soul() -> crate::Result<()> {
    let path = soul_path();
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, DEFAULT_SOUL)?;
    debug!(path = %path.display(), "wrote default SOUL.md");
    Ok(())
}

/// Load AGENTS.md from the workspace root (`data_dir`) if present and non-empty.
pub fn load_agents_md() -> Option<String> {
    load_workspace_markdown(agents_path())
}

/// Load AGENTS.md for a specific agent, falling back to the root file.
pub fn load_agents_md_for_agent(agent_id: &str) -> Option<String> {
    let agent_path = agent_workspace_dir(agent_id).join("AGENTS.md");
    load_workspace_markdown(agent_path).or_else(load_agents_md)
}

/// Load BOOT.md from the workspace root (`data_dir`) if present and non-empty.
pub fn load_boot_md() -> Option<String> {
    load_workspace_markdown(boot_path())
}

/// Load BOOT.md for a specific agent, falling back to the root file.
pub fn load_boot_md_for_agent(agent_id: &str) -> Option<String> {
    let agent_path = agent_workspace_dir(agent_id).join("BOOT.md");
    load_workspace_markdown(agent_path).or_else(load_boot_md)
}

/// Load TOOLS.md from the workspace root (`data_dir`) if present and non-empty.
pub fn load_tools_md() -> Option<String> {
    load_workspace_markdown(tools_path())
}

/// Load TOOLS.md for a specific agent, falling back to the root file.
pub fn load_tools_md_for_agent(agent_id: &str) -> Option<String> {
    let agent_path = agent_workspace_dir(agent_id).join("TOOLS.md");
    load_workspace_markdown(agent_path).or_else(load_tools_md)
}

/// Load GUIDELINES.md from the docs/moltis directory if present and non-empty.
pub fn load_guidelines_md() -> Option<String> {
    load_workspace_markdown(guidelines_path())
}

/// Load GUIDELINES.md for a specific agent, falling back to the root file.
pub fn load_guidelines_md_for_agent(agent_id: &str) -> Option<String> {
    let agent_path = agent_workspace_dir(agent_id).join("GUIDELINES.md");
    load_workspace_markdown(agent_path).or_else(load_guidelines_md)
}

/// Load HEARTBEAT.md from the workspace root (`data_dir`) if present and non-empty.
pub fn load_heartbeat_md() -> Option<String> {
    load_workspace_markdown(heartbeat_path())
}

/// Load MEMORY.md from the workspace root (`data_dir`) if present and non-empty.
pub fn load_memory_md() -> Option<String> {
    load_workspace_markdown(memory_path())
}

/// Load MEMORY.md for a specific agent workspace.
///
/// For `"main"`, this checks `data_dir()/agents/main/MEMORY.md` first and
/// falls back to the root `MEMORY.md`.
pub fn load_memory_md_for_agent(agent_id: &str) -> Option<String> {
    load_memory_md_for_agent_with_source(agent_id).map(|loaded| loaded.content)
}

/// Load MEMORY.md for a specific agent workspace and report its resolved path.
///
/// For `"main"`, this checks `data_dir()/agents/main/MEMORY.md` first and
/// falls back to the root `MEMORY.md`.
pub fn load_memory_md_for_agent_with_source(agent_id: &str) -> Option<LoadedWorkspaceMarkdown> {
    if agent_id == "main" {
        let main_path = agent_workspace_dir("main").join("MEMORY.md");
        if let Some(memory) =
            load_workspace_markdown_with_source(main_path, WorkspaceMarkdownSource::AgentWorkspace)
        {
            return Some(memory);
        }
        return load_workspace_markdown_with_source(
            memory_path(),
            WorkspaceMarkdownSource::RootWorkspace,
        );
    }
    load_workspace_markdown_with_source(
        agent_workspace_dir(agent_id).join("MEMORY.md"),
        WorkspaceMarkdownSource::AgentWorkspace,
    )
}

/// Persist SOUL.md in the workspace root (`data_dir`).
///
/// - `Some(non-empty)` writes `SOUL.md` with the given content
/// - `None` or empty writes an empty `SOUL.md` so that `load_soul()`
///   returns `None` without re-seeding the default
pub fn save_soul(soul: Option<&str>) -> crate::Result<PathBuf> {
    let path = soul_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match soul.map(str::trim) {
        Some(content) if !content.is_empty() => {
            std::fs::write(&path, content)?;
        },
        _ => {
            // Write an empty file rather than deleting so `load_soul()`
            // distinguishes "user cleared soul" from "file never existed".
            std::fs::write(&path, "")?;
        },
    }
    Ok(path)
}

/// Persist SOUL.md into an agent's workspace directory.
///
/// For the main agent this writes to `agents/main/SOUL.md` so that
/// `load_soul_for_agent("main")` picks it up on the primary read path.
pub fn save_soul_for_agent(agent_id: &str, soul: Option<&str>) -> crate::Result<PathBuf> {
    let dir = agent_workspace_dir(agent_id);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("SOUL.md");
    match soul.map(str::trim) {
        Some(content) if !content.is_empty() => {
            std::fs::write(&path, content)?;
        },
        _ => {
            std::fs::write(&path, "")?;
        },
    }
    Ok(path)
}

/// Persist identity values to `IDENTITY.md` using YAML frontmatter.
pub fn save_identity(identity: &AgentIdentity) -> crate::Result<PathBuf> {
    let path = identity_path();
    let has_values =
        identity.name.is_some() || identity.emoji.is_some() || identity.theme.is_some();

    if !has_values {
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        return Ok(path);
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut yaml_lines = Vec::new();
    if let Some(name) = identity.name.as_deref() {
        yaml_lines.push(format!("name: {}", yaml_scalar(name)));
    }
    if let Some(emoji) = identity.emoji.as_deref() {
        yaml_lines.push(format!("emoji: {}", yaml_scalar(emoji)));
    }
    if let Some(theme) = identity.theme.as_deref() {
        yaml_lines.push(format!("theme: {}", yaml_scalar(theme)));
    }
    let yaml = yaml_lines.join("\n");
    let content = format!(
        "---\n{}\n---\n\n# IDENTITY.md\n\nThis file is managed by Moltis settings.\n",
        yaml
    );
    std::fs::write(&path, content)?;
    Ok(path)
}

/// Persist identity values for an agent into its workspace directory.
pub fn save_identity_for_agent(agent_id: &str, identity: &AgentIdentity) -> crate::Result<PathBuf> {
    let dir = agent_workspace_dir(agent_id);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("IDENTITY.md");

    let has_values =
        identity.name.is_some() || identity.emoji.is_some() || identity.theme.is_some();

    if !has_values {
        // Write an empty sentinel so load_identity_for_agent won't fall back
        // to a stale root IDENTITY.md on upgraded installs.
        std::fs::write(&path, "")?;
        return Ok(path);
    }

    let mut yaml_lines = Vec::new();
    if let Some(name) = identity.name.as_deref() {
        yaml_lines.push(format!("name: {}", yaml_scalar(name)));
    }
    if let Some(emoji) = identity.emoji.as_deref() {
        yaml_lines.push(format!("emoji: {}", yaml_scalar(emoji)));
    }
    if let Some(theme) = identity.theme.as_deref() {
        yaml_lines.push(format!("theme: {}", yaml_scalar(theme)));
    }

    let content = format!("---\n{}\n---\n", yaml_lines.join("\n"));
    std::fs::write(&path, content)?;
    Ok(path)
}

/// Persist user values to `USER.md` using YAML frontmatter.
pub fn save_user(user: &UserProfile) -> crate::Result<PathBuf> {
    let path = user_path();
    let has_values = user.name.is_some() || user.timezone.is_some() || user.location.is_some();

    if !has_values {
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        return Ok(path);
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut yaml_lines = Vec::new();
    if let Some(name) = user.name.as_deref() {
        yaml_lines.push(format!("name: {}", yaml_scalar(name)));
    }
    if let Some(ref tz) = user.timezone {
        yaml_lines.push(format!("timezone: {}", yaml_scalar(tz.name())));
    }
    if let Some(ref loc) = user.location {
        yaml_lines.push(format!("latitude: {}", loc.latitude));
        yaml_lines.push(format!("longitude: {}", loc.longitude));
        if let Some(ref place) = loc.place {
            yaml_lines.push(format!("location_place: {}", yaml_scalar(place)));
        }
        if let Some(ts) = loc.updated_at {
            yaml_lines.push(format!("location_updated_at: {ts}"));
        }
    }
    let yaml = yaml_lines.join("\n");
    let content = format!(
        "---\n{}\n---\n\n# USER.md\n\nThis file is managed by Moltis settings.\n",
        yaml
    );
    std::fs::write(&path, content)?;
    Ok(path)
}

/// Persist `USER.md` according to the configured write mode.
///
/// When writes are disabled, any existing `USER.md` file is removed and no new
/// file is created.
pub fn save_user_with_mode(
    user: &UserProfile,
    mode: crate::schema::UserProfileWriteMode,
) -> crate::Result<Option<PathBuf>> {
    if mode.allows_explicit_write() {
        return save_user(user).map(Some);
    }

    let path = user_path();
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(None)
}

pub fn extract_yaml_frontmatter(content: &str) -> Option<&str> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }
    let rest = trimmed.strip_prefix("---")?;
    let rest = rest.strip_prefix('\n')?;
    let end = rest.find("\n---")?;
    Some(&rest[..end])
}

fn parse_identity_frontmatter(frontmatter: &str) -> AgentIdentity {
    let mut identity = AgentIdentity::default();
    // Legacy fields for backward compat with old IDENTITY.md files.
    let mut creature: Option<String> = None;
    let mut vibe: Option<String> = None;

    for raw in frontmatter.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value_raw)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = unquote_yaml_scalar(value_raw.trim());
        if value.is_empty() {
            continue;
        }
        match key {
            "name" => identity.name = Some(value.to_string()),
            "emoji" => identity.emoji = Some(value.to_string()),
            "theme" => identity.theme = Some(value.to_string()),
            // Backward compat: compose legacy creature/vibe into theme.
            "creature" => creature = Some(value.to_string()),
            "vibe" => vibe = Some(value.to_string()),
            _ => {},
        }
    }

    // If no explicit `theme` was set, compose from legacy creature/vibe.
    if identity.theme.is_none() {
        let composed = match (vibe, creature) {
            (Some(v), Some(c)) => Some(format!("{v} {c}")),
            (Some(v), None) => Some(v),
            (None, Some(c)) => Some(c),
            (None, None) => None,
        };
        identity.theme = composed;
    }

    identity
}

fn parse_user_frontmatter(frontmatter: &str) -> UserProfile {
    let mut user = UserProfile::default();
    let mut latitude: Option<f64> = None;
    let mut longitude: Option<f64> = None;
    let mut location_updated_at: Option<i64> = None;
    let mut location_place: Option<String> = None;

    for raw in frontmatter.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value_raw)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = unquote_yaml_scalar(value_raw.trim());
        if value.is_empty() {
            continue;
        }
        match key {
            "name" => user.name = Some(value.to_string()),
            "timezone" => {
                if let Ok(tz) = value.parse::<chrono_tz::Tz>() {
                    user.timezone = Some(crate::schema::Timezone::from(tz));
                }
            },
            "latitude" => latitude = value.parse().ok(),
            "longitude" => longitude = value.parse().ok(),
            "location_updated_at" => location_updated_at = value.parse().ok(),
            "location_place" => location_place = Some(value.to_string()),
            _ => {},
        }
    }

    if let (Some(lat), Some(lon)) = (latitude, longitude) {
        user.location = Some(crate::schema::GeoLocation {
            latitude: lat,
            longitude: lon,
            place: location_place,
            updated_at: location_updated_at,
        });
    }

    user
}

fn unquote_yaml_scalar(value: &str) -> &str {
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

fn yaml_scalar(value: &str) -> String {
    if value.contains(':')
        || value.contains('#')
        || value.starts_with(' ')
        || value.ends_with(' ')
        || value.contains('\n')
    {
        format!("'{}'", value.replace('\'', "''"))
    } else {
        value.to_string()
    }
}

pub fn normalize_workspace_markdown_content(content: &str) -> Option<String> {
    let trimmed = strip_leading_html_comments(content).trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn load_workspace_markdown(path: PathBuf) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    normalize_workspace_markdown_content(&content)
}

fn load_workspace_markdown_with_source(
    path: PathBuf,
    source: WorkspaceMarkdownSource,
) -> Option<LoadedWorkspaceMarkdown> {
    load_workspace_markdown(path.clone()).map(|content| LoadedWorkspaceMarkdown {
        content,
        path,
        source,
    })
}

fn load_identity_from_path(path: &Path) -> Option<AgentIdentity> {
    let content = std::fs::read_to_string(path).ok()?;
    let frontmatter = extract_yaml_frontmatter(&content)?;
    let identity = parse_identity_frontmatter(frontmatter);
    if identity.name.is_none() && identity.emoji.is_none() && identity.theme.is_none() {
        None
    } else {
        Some(identity)
    }
}

fn strip_leading_html_comments(content: &str) -> &str {
    let mut rest = content;
    loop {
        let trimmed = rest.trim_start();
        if !trimmed.starts_with("<!--") {
            return trimmed;
        }
        let Some(end) = trimmed.find("-->") else {
            return "";
        };
        rest = &trimmed[end + 3..];
    }
}
