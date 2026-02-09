//! On-demand Dockerfile-based tool image caching.
//!
//! Skills can declare a `dockerfile` field pointing to a Dockerfile in their
//! directory. When a skill with a Dockerfile is invoked, the image is built
//! (if not already cached) and used as the sandbox container image.
//!
//! Images are tagged as `moltis-cache/<skill-name>:<content-hash>` where
//! the hash is the first 12 hex chars of the SHA-256 of the Dockerfile contents.

use std::path::Path;

use {
    anyhow::{Context, Result},
    async_trait::async_trait,
    serde::{Deserialize, Serialize},
    tracing::{debug, info},
};

/// Metadata about a cached tool image.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedImage {
    pub tag: String,
    pub skill_name: String,
    pub size: String,
    pub created: String,
}

/// Trait for building and managing cached tool images.
///
/// Trait-based so alternative backends (e.g. Apple Container) can provide
/// their own implementation.
#[async_trait]
pub trait ImageBuilder: Send + Sync {
    /// Ensure an image exists for the given skill. Builds from the Dockerfile
    /// if not already cached. Returns the full image tag.
    async fn ensure_image(
        &self,
        skill_name: &str,
        dockerfile: &Path,
        context: &Path,
    ) -> Result<String>;

    /// List all cached tool images.
    async fn list_cached(&self) -> Result<Vec<CachedImage>>;

    /// Remove a single cached image by tag.
    async fn remove_cached(&self, tag: &str) -> Result<()>;

    /// Remove all cached tool images.
    async fn prune_all(&self) -> Result<usize>;
}

/// Docker-based image builder using `docker build`.
pub struct DockerImageBuilder;

impl DockerImageBuilder {
    pub fn new() -> Self {
        Self
    }

    /// Compute the image tag for a skill's Dockerfile.
    /// Format: `moltis-cache/<skill-name>:<first-12-of-sha256>`
    pub fn image_tag(skill_name: &str, dockerfile_contents: &[u8]) -> String {
        use std::hash::Hasher;
        // Use a simple hash for the tag — not cryptographic, just for cache keying.
        // We use two rounds of DefaultHasher to get enough bits for 12 hex chars.
        let mut h1 = std::hash::DefaultHasher::new();
        h1.write(dockerfile_contents);
        let hash1 = h1.finish();
        let mut h2 = std::hash::DefaultHasher::new();
        h2.write(&hash1.to_le_bytes());
        h2.write(dockerfile_contents);
        let hash2 = h2.finish();
        let combined = format!("{:016x}{:016x}", hash1, hash2);
        let short = &combined[..12];
        format!("moltis-cache/{skill_name}:{short}")
    }

    /// Check whether a Docker image exists locally.
    async fn image_exists(tag: &str) -> bool {
        tokio::process::Command::new("docker")
            .args(["image", "inspect", tag])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .is_ok_and(|s| s.success())
    }
}

impl Default for DockerImageBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ImageBuilder for DockerImageBuilder {
    async fn ensure_image(
        &self,
        skill_name: &str,
        dockerfile: &Path,
        context: &Path,
    ) -> Result<String> {
        let contents = tokio::fs::read(dockerfile)
            .await
            .with_context(|| format!("reading Dockerfile at {}", dockerfile.display()))?;

        let tag = Self::image_tag(skill_name, &contents);

        if Self::image_exists(&tag).await {
            debug!(tag, "image cache hit");
            return Ok(tag);
        }

        info!(tag, dockerfile = %dockerfile.display(), "building tool image");

        let output = tokio::process::Command::new("docker")
            .args([
                "build",
                "-t",
                &tag,
                "-f",
                &dockerfile.display().to_string(),
                &context.display().to_string(),
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .await
            .context("failed to run docker build")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("docker build failed for {tag}: {}", stderr.trim());
        }

        info!(tag, "tool image built successfully");
        Ok(tag)
    }

    async fn list_cached(&self) -> Result<Vec<CachedImage>> {
        let output = tokio::process::Command::new("docker")
            .args([
                "images",
                "--filter",
                "reference=moltis-cache/*",
                "--format",
                "{{.Repository}}:{{.Tag}}\t{{.Size}}\t{{.CreatedSince}}",
            ])
            .output()
            .await
            .context("failed to list docker images")?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let images = stdout
            .lines()
            .filter(|l| !l.is_empty())
            .filter_map(|line| {
                let parts: Vec<&str> = line.split('\t').collect();
                if parts.len() < 3 {
                    return None;
                }
                let tag = parts[0].to_string();
                // Extract skill name from "moltis-cache/<skill-name>:<hash>"
                let skill_name = tag
                    .strip_prefix("moltis-cache/")
                    .and_then(|s| s.split(':').next())
                    .unwrap_or("")
                    .to_string();
                Some(CachedImage {
                    tag,
                    skill_name,
                    size: parts[1].to_string(),
                    created: parts[2].to_string(),
                })
            })
            .collect();

        Ok(images)
    }

    async fn remove_cached(&self, tag: &str) -> Result<()> {
        // Only allow removing moltis-cache images.
        if !tag.starts_with("moltis-cache/") {
            anyhow::bail!("refusing to remove non-cache image: {tag}");
        }

        let output = tokio::process::Command::new("docker")
            .args(["rmi", tag])
            .output()
            .await
            .context("failed to run docker rmi")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("docker rmi failed for {tag}: {}", stderr.trim());
        }

        info!(tag, "removed cached image");
        Ok(())
    }

    async fn prune_all(&self) -> Result<usize> {
        let images = self.list_cached().await?;
        let count = images.len();
        for img in &images {
            if let Err(e) = self.remove_cached(&img.tag).await {
                tracing::warn!(tag = img.tag, "failed to prune: {e}");
            }
        }
        Ok(count)
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_image_tag_format() {
        let tag =
            DockerImageBuilder::image_tag("my-skill", b"FROM ubuntu:25.10\nRUN apt-get update\n");
        assert!(tag.starts_with("moltis-cache/my-skill:"));
        // Hash portion is 12 hex chars
        let hash_part = tag.strip_prefix("moltis-cache/my-skill:").unwrap();
        assert_eq!(hash_part.len(), 12);
        assert!(hash_part.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_image_tag_deterministic() {
        let a = DockerImageBuilder::image_tag("skill", b"FROM alpine\n");
        let b = DockerImageBuilder::image_tag("skill", b"FROM alpine\n");
        assert_eq!(a, b);
    }

    #[test]
    fn test_image_tag_different_content() {
        let a = DockerImageBuilder::image_tag("skill", b"FROM alpine\n");
        let b = DockerImageBuilder::image_tag("skill", b"FROM ubuntu\n");
        assert_ne!(a, b);
    }

    #[test]
    fn test_image_tag_different_skill() {
        let a = DockerImageBuilder::image_tag("skill-a", b"FROM alpine\n");
        let b = DockerImageBuilder::image_tag("skill-b", b"FROM alpine\n");
        // Same hash, different skill name prefix
        assert_ne!(a, b);
        assert!(a.starts_with("moltis-cache/skill-a:"));
        assert!(b.starts_with("moltis-cache/skill-b:"));
    }

    #[test]
    fn test_cached_image_serde() {
        let img = CachedImage {
            tag: "moltis-cache/my-skill:abc123def456".into(),
            skill_name: "my-skill".into(),
            size: "150MB".into(),
            created: "2 hours ago".into(),
        };
        let json = serde_json::to_string(&img).unwrap();
        let back: CachedImage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.tag, img.tag);
        assert_eq!(back.skill_name, img.skill_name);
    }
}
