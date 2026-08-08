//! Discover MCP servers from an already materialized repository directory.

use std::{
    collections::{BTreeMap, HashMap},
    path::{Component, Path, PathBuf},
};

use {
    secrecy::Secret,
    serde::{Deserialize, Serialize},
    serde_json::{Map, Value},
};

use crate::{
    error::{Context, Error, Result},
    registry::{McpServerConfig, TransportType},
    remote::environment_placeholder_names,
};

const MOLTIS_MANIFEST: &str = ".moltis/mcp-repository.json";
const MARKETPLACE_MANIFEST: &str = ".claude-plugin/marketplace.json";
const ROOT_MCP_MANIFEST: &str = ".mcp.json";
const CLAUDE_PLUGIN_MANIFEST: &str = ".claude-plugin/plugin.json";
const CODEX_PLUGIN_MANIFEST: &str = ".codex-plugin/plugin.json";

/// The repository manifest format selected during discovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepositoryManifestKind {
    Moltis,
    ClaudeMarketplace,
    ClaudeMcp,
}

/// Identity inherited from a Claude marketplace plugin entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginIdentity {
    pub marketplace_name: Option<String>,
    pub plugin_name: String,
    pub manifest_name: Option<String>,
}

/// A server configuration with repository provenance.
#[derive(Debug, Clone)]
pub struct DiscoveredMcpServer {
    pub name: String,
    pub config: McpServerConfig,
    pub source_manifest_path: PathBuf,
    pub plugin_root: Option<PathBuf>,
    pub plugin: Option<PluginIdentity>,
}

/// Classification for non-fatal discovery diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoveryWarningKind {
    UnsupportedField,
    UnsupportedPluginSource,
    UnsafePath,
    MutableLatest,
    TlsVerificationDisabled,
    UnboundEnvironmentPlaceholder,
    LikelyLiteralSecret,
}

/// A non-fatal issue found while inspecting repository manifests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryWarning {
    pub kind: DiscoveryWarningKind,
    pub message: String,
    pub source_manifest_path: PathBuf,
    pub plugin_name: Option<String>,
    pub server_name: Option<String>,
}

/// Complete result of repository discovery.
#[derive(Debug, Clone)]
pub struct RepositoryDiscovery {
    pub repository_root: PathBuf,
    pub source_manifest_path: Option<PathBuf>,
    pub manifest_kind: Option<RepositoryManifestKind>,
    pub servers: Vec<DiscoveredMcpServer>,
    pub warnings: Vec<DiscoveryWarning>,
}

#[derive(Debug, Deserialize)]
struct McpManifest {
    #[serde(default, rename = "mcpServers")]
    mcp_servers: BTreeMap<String, RawServerConfig>,
    #[serde(default)]
    version: Option<u32>,
    #[serde(default)]
    plugins: Vec<MoltisRepositoryPlugin>,
}

#[derive(Debug, Deserialize)]
struct MoltisRepositoryPlugin {
    root: String,
}

#[derive(Debug, Deserialize)]
struct MarketplaceManifest {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    metadata: MarketplaceMetadata,
    #[serde(default)]
    plugins: Vec<MarketplacePlugin>,
}

#[derive(Debug, Default, Deserialize)]
struct MarketplaceMetadata {
    #[serde(default, rename = "pluginRoot")]
    plugin_root: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MarketplacePlugin {
    name: String,
    source: Value,
    #[serde(default, rename = "mcpServers")]
    mcp_servers: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct PluginManifest {
    #[serde(default)]
    name: Option<String>,
    #[serde(default, rename = "mcpServers")]
    mcp_servers: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct RawServerConfig {
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default, rename = "type")]
    server_type: Option<String>,
    #[serde(default)]
    transport: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

/// Discover MCP servers without cloning or otherwise modifying `repository_dir`.
pub fn discover_repository(repository_dir: &Path) -> Result<RepositoryDiscovery> {
    let repository_root = repository_dir.canonicalize().with_context(|| {
        format!(
            "failed to resolve MCP repository directory: {}",
            repository_dir.display()
        )
    })?;
    if !repository_root.is_dir() {
        return Err(Error::message(format!(
            "MCP repository path is not a directory: {}",
            repository_root.display()
        )));
    }

    let candidates = [
        (MOLTIS_MANIFEST, RepositoryManifestKind::Moltis),
        (
            MARKETPLACE_MANIFEST,
            RepositoryManifestKind::ClaudeMarketplace,
        ),
        (ROOT_MCP_MANIFEST, RepositoryManifestKind::ClaudeMcp),
    ];
    let selected = candidates
        .into_iter()
        .find(|(relative, _)| repository_root.join(relative).is_file());
    let Some((relative, kind)) = selected else {
        return Ok(RepositoryDiscovery {
            repository_root,
            source_manifest_path: None,
            manifest_kind: None,
            servers: Vec::new(),
            warnings: Vec::new(),
        });
    };

    let manifest_path = safe_existing_file(&repository_root, &repository_root.join(relative))?;
    let mut discovery = RepositoryDiscovery {
        repository_root: repository_root.clone(),
        source_manifest_path: Some(manifest_path.clone()),
        manifest_kind: Some(kind),
        servers: Vec::new(),
        warnings: Vec::new(),
    };

    match kind {
        RepositoryManifestKind::Moltis => {
            let manifest: McpManifest = read_json(&manifest_path)?;
            discover_moltis_plugins(&mut discovery, &manifest, &manifest_path)?;
            add_servers(
                &mut discovery,
                manifest.mcp_servers,
                &manifest_path,
                None,
                None,
            );
        },
        RepositoryManifestKind::ClaudeMcp => {
            let manifest: McpManifest = read_json(&manifest_path)?;
            add_servers(
                &mut discovery,
                manifest.mcp_servers,
                &manifest_path,
                None,
                None,
            );
        },
        RepositoryManifestKind::ClaudeMarketplace => {
            discover_marketplace(&mut discovery, &manifest_path)?;
        },
    }

    Ok(discovery)
}

fn discover_moltis_plugins(
    discovery: &mut RepositoryDiscovery,
    manifest: &McpManifest,
    manifest_path: &Path,
) -> Result<()> {
    if manifest.version.is_some_and(|version| version != 1) {
        return Err(Error::message(format!(
            "unsupported Moltis MCP repository manifest version in {}",
            manifest_path.display()
        )));
    }
    for plugin in &manifest.plugins {
        let plugin_root = safe_existing_dir(&discovery.repository_root, &plugin.root)?;
        let mcp_manifest_path = plugin_root.join(ROOT_MCP_MANIFEST);
        if !mcp_manifest_path.is_file() {
            continue;
        }
        let mcp_manifest_path = safe_existing_file(&plugin_root, &mcp_manifest_path)?;
        let plugin_manifest: McpManifest = read_json(&mcp_manifest_path)?;
        add_servers(
            discovery,
            plugin_manifest.mcp_servers,
            &mcp_manifest_path,
            Some(&plugin_root),
            None,
        );
    }
    Ok(())
}

fn discover_marketplace(discovery: &mut RepositoryDiscovery, manifest_path: &Path) -> Result<()> {
    let marketplace: MarketplaceManifest = read_json(manifest_path)?;
    let marketplace_name = marketplace.name;
    let base_root = match marketplace.metadata.plugin_root {
        Some(relative) => {
            let declared = match safe_relative_join(&discovery.repository_root, &relative) {
                Ok(path) => path,
                Err(error) => {
                    discovery.warnings.push(warning(
                        DiscoveryWarningKind::UnsafePath,
                        format!("ignoring unsafe marketplace metadata.pluginRoot: {error}"),
                        manifest_path,
                        None,
                        None,
                    ));
                    return Ok(());
                },
            };
            if !declared.exists() {
                declared
            } else {
                match safe_existing_dir(&discovery.repository_root, &relative) {
                    Ok(path) => path,
                    Err(error) => {
                        discovery.warnings.push(warning(
                            DiscoveryWarningKind::UnsafePath,
                            format!("ignoring unsafe marketplace metadata.pluginRoot: {error}"),
                            manifest_path,
                            None,
                            None,
                        ));
                        return Ok(());
                    },
                }
            }
        },
        None => discovery.repository_root.clone(),
    };

    for plugin in marketplace.plugins {
        let plugin_name = plugin.name.trim().to_string();
        if plugin_name.is_empty() {
            discovery.warnings.push(warning(
                DiscoveryWarningKind::UnsupportedField,
                "ignoring marketplace plugin with an empty name".to_string(),
                manifest_path,
                None,
                None,
            ));
            continue;
        }
        let Some(source) = plugin.source.as_str() else {
            discovery.warnings.push(warning(
                DiscoveryWarningKind::UnsupportedPluginSource,
                "only materialized local marketplace plugin sources are supported".to_string(),
                manifest_path,
                Some(&plugin_name),
                None,
            ));
            continue;
        };
        let declared_plugin_root = match safe_relative_join(&base_root, source) {
            Ok(path) if path.starts_with(&discovery.repository_root) => path,
            Ok(_) => {
                discovery.warnings.push(warning(
                    DiscoveryWarningKind::UnsafePath,
                    "plugin source resolves outside the repository".to_string(),
                    manifest_path,
                    Some(&plugin_name),
                    None,
                ));
                continue;
            },
            Err(error) => {
                discovery.warnings.push(warning(
                    DiscoveryWarningKind::UnsafePath,
                    format!("ignoring unsafe plugin source: {error}"),
                    manifest_path,
                    Some(&plugin_name),
                    None,
                ));
                continue;
            },
        };
        let plugin_root = match safe_existing_dir(&base_root, source) {
            Ok(path) if path.starts_with(&discovery.repository_root) => path,
            Ok(_) => {
                discovery.warnings.push(warning(
                    DiscoveryWarningKind::UnsafePath,
                    "plugin source resolves outside the repository".to_string(),
                    manifest_path,
                    Some(&plugin_name),
                    None,
                ));
                continue;
            },
            Err(error) => {
                if plugin.mcp_servers.is_some() && !declared_plugin_root.exists() {
                    declared_plugin_root
                } else if !declared_plugin_root.exists() {
                    // Sparse materialization intentionally omits non-MCP marketplace roots.
                    continue;
                } else {
                    discovery.warnings.push(warning(
                        DiscoveryWarningKind::UnsafePath,
                        format!("ignoring unsafe plugin source: {error}"),
                        manifest_path,
                        Some(&plugin_name),
                        None,
                    ));
                    continue;
                }
            },
        };

        let mut identity = PluginIdentity {
            marketplace_name: marketplace_name.clone(),
            plugin_name: plugin_name.clone(),
            manifest_name: None,
        };
        let mut plugin_declares_mcp = false;
        for relative in [CLAUDE_PLUGIN_MANIFEST, CODEX_PLUGIN_MANIFEST] {
            let plugin_manifest_path = plugin_root.join(relative);
            if !plugin_manifest_path.is_file() {
                continue;
            }
            let safe_path = match safe_existing_file(&plugin_root, &plugin_manifest_path) {
                Ok(path) => path,
                Err(error) => {
                    discovery.warnings.push(warning(
                        DiscoveryWarningKind::UnsafePath,
                        format!("ignoring unsafe plugin manifest: {error}"),
                        manifest_path,
                        Some(&plugin_name),
                        None,
                    ));
                    continue;
                },
            };
            let plugin_manifest: PluginManifest = read_json(&safe_path)?;
            identity.manifest_name = plugin_manifest.name;
            if let Some(component) = plugin_manifest.mcp_servers {
                plugin_declares_mcp = true;
                add_component_value(discovery, &component, &safe_path, &plugin_root, &identity)?;
                break;
            }
        }

        if let Some(inline) = plugin.mcp_servers {
            add_component_value(discovery, &inline, manifest_path, &plugin_root, &identity)?;
        }

        let default_manifest_path = plugin_root.join(ROOT_MCP_MANIFEST);
        if !plugin_declares_mcp && default_manifest_path.is_file() {
            let safe_path = match safe_existing_file(&plugin_root, &default_manifest_path) {
                Ok(path) => path,
                Err(error) => {
                    discovery.warnings.push(warning(
                        DiscoveryWarningKind::UnsafePath,
                        format!("ignoring unsafe plugin MCP manifest: {error}"),
                        manifest_path,
                        Some(&plugin_name),
                        None,
                    ));
                    continue;
                },
            };
            let manifest: McpManifest = read_json(&safe_path)?;
            add_servers(
                discovery,
                manifest.mcp_servers,
                &safe_path,
                Some(&plugin_root),
                Some(&identity),
            );
        }
    }
    Ok(())
}

fn add_component_value(
    discovery: &mut RepositoryDiscovery,
    component: &Value,
    declaring_manifest: &Path,
    plugin_root: &Path,
    identity: &PluginIdentity,
) -> Result<()> {
    match component {
        Value::String(relative) => {
            let component_path = match safe_existing_file(plugin_root, &plugin_root.join(relative))
            {
                Ok(path) => path,
                Err(error) => {
                    discovery.warnings.push(warning(
                        DiscoveryWarningKind::UnsafePath,
                        format!("ignoring unsafe mcpServers path: {error}"),
                        declaring_manifest,
                        Some(&identity.plugin_name),
                        None,
                    ));
                    return Ok(());
                },
            };
            let manifest: McpManifest = read_json(&component_path)?;
            add_servers(
                discovery,
                manifest.mcp_servers,
                &component_path,
                Some(plugin_root),
                Some(identity),
            );
        },
        Value::Array(components) => {
            for component in components {
                add_component_value(
                    discovery,
                    component,
                    declaring_manifest,
                    plugin_root,
                    identity,
                )?;
            }
        },
        Value::Object(object) => {
            let servers = parse_inline_servers(object, declaring_manifest)?;
            add_servers(
                discovery,
                servers,
                declaring_manifest,
                Some(plugin_root),
                Some(identity),
            );
        },
        _ => discovery.warnings.push(warning(
            DiscoveryWarningKind::UnsupportedField,
            "mcpServers must be an object, path, or array of paths".to_string(),
            declaring_manifest,
            Some(&identity.plugin_name),
            None,
        )),
    }
    Ok(())
}

fn parse_inline_servers(
    object: &Map<String, Value>,
    source: &Path,
) -> Result<BTreeMap<String, RawServerConfig>> {
    serde_json::from_value(Value::Object(object.clone())).with_context(|| {
        format!(
            "failed to parse inline MCP servers from {}",
            source.display()
        )
    })
}

fn add_servers(
    discovery: &mut RepositoryDiscovery,
    servers: BTreeMap<String, RawServerConfig>,
    source_manifest_path: &Path,
    plugin_root: Option<&Path>,
    plugin: Option<&PluginIdentity>,
) {
    for (name, raw) in servers {
        let trimmed_name = name.trim();
        if trimmed_name.is_empty() {
            discovery.warnings.push(warning(
                DiscoveryWarningKind::UnsupportedField,
                "ignoring MCP server with an empty name".to_string(),
                source_manifest_path,
                plugin.map(|value| value.plugin_name.as_str()),
                None,
            ));
            continue;
        }
        emit_server_warnings(
            discovery,
            trimmed_name,
            &raw,
            source_manifest_path,
            plugin_root,
            plugin,
        );
        let config = to_server_config(&discovery.repository_root, plugin_root, raw);
        discovery.servers.push(DiscoveredMcpServer {
            name: trimmed_name.to_string(),
            config,
            source_manifest_path: source_manifest_path.to_path_buf(),
            plugin_root: plugin_root.map(Path::to_path_buf),
            plugin: plugin.cloned(),
        });
    }
}

fn to_server_config(
    repository_root: &Path,
    plugin_root: Option<&Path>,
    raw: RawServerConfig,
) -> McpServerConfig {
    let transport_name = raw
        .server_type
        .as_deref()
        .or(raw.transport.as_deref())
        .unwrap_or(if raw.url.is_some() {
            "http"
        } else {
            "stdio"
        });
    let transport = match transport_name.to_ascii_lowercase().as_str() {
        "sse" => TransportType::Sse,
        "http" | "streamable-http" | "streamable_http" | "streamablehttp" | "remote" => {
            TransportType::StreamableHttp
        },
        _ => TransportType::Stdio,
    };
    let args = raw
        .args
        .into_iter()
        .map(|arg| expand_plugin_root(&arg, plugin_root))
        .collect();
    let stdio = matches!(transport, TransportType::Stdio);
    McpServerConfig {
        command: raw.command.unwrap_or_default(),
        args,
        cwd: stdio.then(|| repository_root.to_path_buf()),
        env: if stdio {
            secret_map(raw.env)
        } else {
            HashMap::new()
        },
        enabled: false,
        transport,
        url: raw.url.map(Secret::new),
        headers: if stdio {
            HashMap::new()
        } else {
            secret_map(raw.headers)
        },
        ..Default::default()
    }
}

fn secret_map(values: BTreeMap<String, String>) -> HashMap<String, Secret<String>> {
    values
        .into_iter()
        .map(|(key, value)| (key, Secret::new(value)))
        .collect()
}

fn expand_plugin_root(value: &str, plugin_root: Option<&Path>) -> String {
    let Some(plugin_root) = plugin_root else {
        return value.to_string();
    };
    let root = plugin_root.to_string_lossy();
    value
        .replace("${CLAUDE_PLUGIN_ROOT}", &root)
        .replace("$CLAUDE_PLUGIN_ROOT", &root)
}

fn emit_server_warnings(
    discovery: &mut RepositoryDiscovery,
    server_name: &str,
    raw: &RawServerConfig,
    source: &Path,
    plugin_root: Option<&Path>,
    plugin: Option<&PluginIdentity>,
) {
    let plugin_name = plugin.map(|value| value.plugin_name.as_str());
    for field in raw.extra.keys() {
        discovery.warnings.push(warning(
            DiscoveryWarningKind::UnsupportedField,
            format!("MCP server field '{field}' is not supported"),
            source,
            plugin_name,
            Some(server_name),
        ));
    }
    if raw
        .server_type
        .as_deref()
        .or(raw.transport.as_deref())
        .is_some_and(|value| !supported_transport(value))
    {
        discovery.warnings.push(warning(
            DiscoveryWarningKind::UnsupportedField,
            "MCP server transport value is not supported".to_string(),
            source,
            plugin_name,
            Some(server_name),
        ));
    }
    if raw
        .command
        .iter()
        .chain(raw.args.iter())
        .any(|value| value.to_ascii_lowercase().contains("@latest"))
    {
        discovery.warnings.push(warning(
            DiscoveryWarningKind::MutableLatest,
            "MCP server references a mutable @latest package".to_string(),
            source,
            plugin_name,
            Some(server_name),
        ));
    }
    if tls_verification_disabled(raw) {
        discovery.warnings.push(warning(
            DiscoveryWarningKind::TlsVerificationDisabled,
            "MCP server disables TLS certificate verification".to_string(),
            source,
            plugin_name,
            Some(server_name),
        ));
    }
    let contains_environment_placeholder = raw
        .command
        .iter()
        .chain(raw.args.iter())
        .map(|value| expand_plugin_root(value, plugin_root))
        .chain(raw.env.values().cloned())
        .chain(raw.headers.values().cloned())
        .chain(raw.url.iter().cloned())
        .any(|value| !environment_placeholder_names(&value).is_empty());
    if contains_environment_placeholder {
        discovery.warnings.push(warning(
            DiscoveryWarningKind::UnboundEnvironmentPlaceholder,
            "MCP server references an environment placeholder without a managed secret binding"
                .to_string(),
            source,
            plugin_name,
            Some(server_name),
        ));
    }
    for (key, value) in raw.env.iter().chain(raw.headers.iter()) {
        if likely_secret_name(key) && literal_value(value) || looks_like_literal_secret(value) {
            discovery.warnings.push(warning(
                DiscoveryWarningKind::LikelyLiteralSecret,
                format!("MCP server field '{key}' appears to contain a literal secret"),
                source,
                plugin_name,
                Some(server_name),
            ));
        }
    }
    if raw
        .command
        .iter()
        .chain(raw.args.iter())
        .any(|value| looks_like_literal_secret(value))
        || raw.url.as_deref().is_some_and(url_contains_literal_secret)
        || args_contain_literal_secret(&raw.args)
    {
        discovery.warnings.push(warning(
            DiscoveryWarningKind::LikelyLiteralSecret,
            "MCP server URL or arguments appear to contain a literal secret".to_string(),
            source,
            plugin_name,
            Some(server_name),
        ));
    }
}

fn supported_transport(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "stdio"
            | "sse"
            | "http"
            | "streamable-http"
            | "streamable_http"
            | "streamablehttp"
            | "remote"
    )
}

fn tls_verification_disabled(raw: &RawServerConfig) -> bool {
    raw.env
        .get("NODE_TLS_REJECT_UNAUTHORIZED")
        .is_some_and(|value| value == "0")
        || raw.extra.iter().any(|(key, value)| {
            let normalized = key.to_ascii_lowercase().replace(['_', '-'], "");
            matches!(normalized.as_str(), "tlsskipverify" | "insecure")
                && value.as_bool() == Some(true)
                || matches!(normalized.as_str(), "rejectunauthorized" | "strictssl")
                    && value.as_bool() == Some(false)
        })
}

fn likely_secret_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    [
        "SECRET",
        "TOKEN",
        "PASSWORD",
        "API_KEY",
        "API-KEY",
        "AUTHORIZATION",
    ]
    .iter()
    .any(|marker| upper.contains(marker))
        || upper.ends_with("_KEY")
}

fn literal_value(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && !trimmed.contains("${")
        && !trimmed.starts_with('$')
        && !trimmed.starts_with('<')
}

fn url_contains_literal_secret(value: &str) -> bool {
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    (!url.username().is_empty() || url.password().is_some())
        || url.path_segments().is_some_and(|segments| {
            segments
                .filter(|segment| !segment.is_empty())
                .any(looks_like_literal_secret)
        })
        || url.query_pairs().any(|(key, value)| {
            likely_secret_name(&key) && literal_value(&value) || looks_like_literal_secret(&value)
        })
}

fn args_contain_literal_secret(args: &[String]) -> bool {
    args.windows(2).any(|pair| {
        let flag = pair[0].trim_start_matches('-').replace('-', "_");
        likely_secret_name(&flag) && literal_value(&pair[1])
    }) || args.iter().any(|arg| {
        arg.split_once('=').is_some_and(|(flag, value)| {
            likely_secret_name(&flag.trim_start_matches('-').replace('-', "_"))
                && literal_value(value)
        })
    })
}

fn looks_like_literal_secret(value: &str) -> bool {
    let trimmed = value.trim();
    if !literal_value(trimmed) {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    lower.starts_with("bearer ")
        || lower.starts_with("basic ")
        || [
            "sk-",
            "sk_",
            "ghp_",
            "github_pat_",
            "xoxb-",
            "xoxp-",
            "xapp-",
        ]
        .iter()
        .any(|prefix| lower.starts_with(prefix))
        || trimmed.starts_with("AKIA") && trimmed.len() >= 16
        || trimmed.starts_with("eyJ") && trimmed.matches('.').count() == 2
}

fn safe_existing_dir(base: &Path, relative: &str) -> Result<PathBuf> {
    let path = safe_relative_join(base, relative)?;
    let canonical = path
        .canonicalize()
        .with_context(|| format!("failed to resolve path: {}", path.display()))?;
    if !canonical.starts_with(base) || !canonical.is_dir() {
        return Err(Error::message(format!(
            "path is not a directory within {}: {}",
            base.display(),
            canonical.display()
        )));
    }
    Ok(canonical)
}

fn safe_existing_file(base: &Path, path: &Path) -> Result<PathBuf> {
    let canonical = path
        .canonicalize()
        .with_context(|| format!("failed to resolve path: {}", path.display()))?;
    if !canonical.starts_with(base) || !canonical.is_file() {
        return Err(Error::message(format!(
            "path is not a file within {}: {}",
            base.display(),
            canonical.display()
        )));
    }
    Ok(canonical)
}

fn safe_relative_join(base: &Path, relative: &str) -> Result<PathBuf> {
    let relative = if relative.trim().is_empty() {
        "."
    } else {
        relative.trim()
    };
    let mut result = base.to_path_buf();
    for component in Path::new(relative).components() {
        match component {
            Component::CurDir => {},
            Component::Normal(segment) => result.push(segment),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(Error::message(format!(
                    "path traversal is not allowed: {relative}"
                )));
            },
        }
    }
    Ok(result)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read MCP manifest: {}", path.display()))?;
    serde_json::from_str(&content)
        .with_context(|| format!("failed to parse MCP manifest: {}", path.display()))
}

fn warning(
    kind: DiscoveryWarningKind,
    message: String,
    source: &Path,
    plugin_name: Option<&str>,
    server_name: Option<&str>,
) -> DiscoveryWarning {
    DiscoveryWarning {
        kind,
        message,
        source_manifest_path: source.to_path_buf(),
        plugin_name: plugin_name.map(String::from),
        server_name: server_name.map(String::from),
    }
}

#[cfg(test)]
mod tests {
    use {super::*, secrecy::ExposeSecret};

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/yolo-marketplace")
    }

    #[test]
    fn discovers_eight_marketplace_plugins_and_inline_entries_only() -> Result<()> {
        let discovery = discover_repository(&fixture_path())?;

        assert_eq!(
            discovery.manifest_kind,
            Some(RepositoryManifestKind::ClaudeMarketplace)
        );
        assert_eq!(discovery.servers.len(), 10);
        assert!(discovery.warnings.is_empty());
        assert!(
            discovery
                .servers
                .iter()
                .all(|server| !server.config.enabled)
        );
        for index in 1..=8 {
            let name = format!("plugin-{index}");
            let server = discovery
                .servers
                .iter()
                .find(|server| server.name == name)
                .ok_or_else(|| Error::message(format!("missing fixture server {name}")))?;
            assert_eq!(
                server
                    .plugin
                    .as_ref()
                    .map(|plugin| plugin.plugin_name.as_str()),
                Some(name.as_str())
            );
            assert_eq!(
                server
                    .plugin
                    .as_ref()
                    .and_then(|plugin| plugin.marketplace_name.as_deref()),
                Some("yolo-marketplace")
            );
            assert_eq!(
                server
                    .plugin
                    .as_ref()
                    .and_then(|plugin| plugin.manifest_name.as_deref()),
                Some(name.as_str())
            );
            assert!(server.source_manifest_path.ends_with(".mcp.json"));
        }
        assert!(
            discovery
                .servers
                .iter()
                .any(|server| server.name == "inline-stdio")
        );
        assert!(
            discovery
                .servers
                .iter()
                .any(|server| server.name == "inline-http")
        );
        assert!(
            !discovery
                .servers
                .iter()
                .any(|server| server.name == "unrelated")
        );
        assert!(
            !discovery
                .servers
                .iter()
                .any(|server| server.name == "root-ignored-by-marketplace")
        );
        Ok(())
    }

    #[test]
    fn moltis_manifest_wins_and_parses_aliases_and_warnings() -> Result<()> {
        let temp = tempfile::tempdir()?;
        std::fs::create_dir_all(temp.path().join(".moltis"))?;
        std::fs::write(
            temp.path().join(MOLTIS_MANIFEST),
            r#"{
                "mcpServers": {
                    "local": {
                        "command": "npx",
                        "args": ["-y", "pkg@latest"],
                        "env": {
                            "API_KEY": "literal-value",
                            "NODE_TLS_REJECT_UNAUTHORIZED": "0"
                        },
                        "cwd": "/tmp"
                    },
                    "remote": {
                        "type": "remote",
                        "url": "https://example.test/mcp",
                        "headers": {"Authorization": "${MCP_TOKEN}"}
                    }
                }
            }"#,
        )?;
        std::fs::write(
            temp.path().join(ROOT_MCP_MANIFEST),
            r#"{"mcpServers":{"ignored":{"command":"ignored"}}}"#,
        )?;

        let discovery = discover_repository(temp.path())?;
        assert_eq!(
            discovery.manifest_kind,
            Some(RepositoryManifestKind::Moltis)
        );
        assert_eq!(discovery.servers.len(), 2);
        let local = discovery
            .servers
            .iter()
            .find(|server| server.name == "local")
            .ok_or_else(|| Error::message("missing local server"))?;
        assert_eq!(
            local.config.cwd.as_deref(),
            Some(discovery.repository_root.as_path())
        );
        let remote = discovery
            .servers
            .iter()
            .find(|server| server.name == "remote")
            .ok_or_else(|| Error::message("missing remote server"))?;
        assert_eq!(remote.config.transport, TransportType::StreamableHttp);
        assert!(remote.config.cwd.is_none());
        assert_eq!(
            remote
                .config
                .headers
                .get("Authorization")
                .map(ExposeSecret::expose_secret)
                .map(String::as_str),
            Some("${MCP_TOKEN}")
        );
        for kind in [
            DiscoveryWarningKind::UnsupportedField,
            DiscoveryWarningKind::MutableLatest,
            DiscoveryWarningKind::TlsVerificationDisabled,
            DiscoveryWarningKind::UnboundEnvironmentPlaceholder,
            DiscoveryWarningKind::LikelyLiteralSecret,
        ] {
            assert!(
                discovery
                    .warnings
                    .iter()
                    .any(|warning| warning.kind == kind)
            );
        }
        Ok(())
    }

    #[test]
    fn discovers_moltis_declared_plugin_roots() -> Result<()> {
        let temp = tempfile::tempdir()?;
        std::fs::create_dir_all(temp.path().join(".moltis"))?;
        std::fs::create_dir_all(temp.path().join("plugins/native"))?;
        std::fs::write(
            temp.path().join(MOLTIS_MANIFEST),
            r#"{"version":1,"plugins":[{"root":"plugins/native"}]}"#,
        )?;
        std::fs::write(
            temp.path().join("plugins/native/.mcp.json"),
            r#"{"mcpServers":{"native":{"command":"native-server"}}}"#,
        )?;

        let discovery = discover_repository(temp.path())?;

        assert_eq!(discovery.servers.len(), 1);
        assert_eq!(discovery.servers[0].name, "native");
        assert!(
            discovery.servers[0]
                .source_manifest_path
                .ends_with("plugins/native/.mcp.json")
        );
        Ok(())
    }

    #[test]
    fn expands_plugin_root_in_args() -> Result<()> {
        let discovery = discover_repository(&fixture_path())?;
        let server = discovery
            .servers
            .iter()
            .find(|server| server.name == "plugin-1")
            .ok_or_else(|| Error::message("missing plugin-1"))?;
        let plugin_root = server
            .plugin_root
            .as_ref()
            .ok_or_else(|| Error::message("missing plugin root"))?
            .to_string_lossy();
        assert_eq!(server.config.args, vec![format!("{plugin_root}/server.js")]);
        Ok(())
    }

    #[test]
    fn rejects_traversal_plugin_sources() -> Result<()> {
        let temp = tempfile::tempdir()?;
        std::fs::create_dir_all(temp.path().join(".claude-plugin"))?;
        std::fs::write(
            temp.path().join(MARKETPLACE_MANIFEST),
            r#"{
                "name": "unsafe",
                "plugins": [{"name": "escape", "source": "../outside"}]
            }"#,
        )?;

        let discovery = discover_repository(temp.path())?;
        assert!(discovery.servers.is_empty());
        assert!(
            discovery
                .warnings
                .iter()
                .any(|warning| warning.kind == DiscoveryWarningKind::UnsafePath)
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_plugin_source_escapes() -> Result<()> {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir()?;
        let outside = tempfile::tempdir()?;
        std::fs::create_dir_all(temp.path().join(".claude-plugin"))?;
        symlink(outside.path(), temp.path().join("escaped"))?;
        std::fs::write(
            temp.path().join(MARKETPLACE_MANIFEST),
            r#"{
                "name": "unsafe",
                "plugins": [{"name": "escape", "source": "./escaped"}]
            }"#,
        )?;

        let discovery = discover_repository(temp.path())?;
        assert!(discovery.servers.is_empty());
        assert!(
            discovery
                .warnings
                .iter()
                .any(|warning| warning.kind == DiscoveryWarningKind::UnsafePath)
        );
        Ok(())
    }

    #[test]
    fn discovers_inline_server_when_sparse_plugin_root_is_absent() -> Result<()> {
        let temp = tempfile::tempdir()?;
        std::fs::create_dir_all(temp.path().join(".claude-plugin"))?;
        std::fs::write(
            temp.path().join(MARKETPLACE_MANIFEST),
            r#"{
                "name":"sparse",
                "metadata":{"pluginRoot":"plugins"},
                "plugins":[
                    {"name":"omitted","source":"omitted"},
                    {
                        "name":"inline",
                        "source":"inline",
                        "mcpServers":{"remote":{"type":"http","url":"https://example.test/mcp"}}
                    }
                ]
            }"#,
        )?;

        let discovery = discover_repository(temp.path())?;

        assert_eq!(discovery.servers.len(), 1);
        assert_eq!(discovery.servers[0].name, "remote");
        assert!(discovery.warnings.is_empty());
        Ok(())
    }

    #[test]
    fn discovers_codex_plugin_manifest_mcp_servers() -> Result<()> {
        let temp = tempfile::tempdir()?;
        std::fs::create_dir_all(temp.path().join(".claude-plugin"))?;
        std::fs::create_dir_all(temp.path().join("plugin/.codex-plugin"))?;
        std::fs::write(
            temp.path().join(MARKETPLACE_MANIFEST),
            r#"{"plugins":[{"name":"codex","source":"plugin"}]}"#,
        )?;
        std::fs::write(
            temp.path().join("plugin/.codex-plugin/plugin.json"),
            r#"{"name":"codex","mcpServers":"./.mcp.json"}"#,
        )?;
        std::fs::write(
            temp.path().join("plugin/.mcp.json"),
            r#"{"mcpServers":{"codex":{"command":"codex-server"}}}"#,
        )?;

        let discovery = discover_repository(temp.path())?;

        assert_eq!(discovery.servers.len(), 1);
        assert_eq!(discovery.servers[0].name, "codex");
        Ok(())
    }
}
