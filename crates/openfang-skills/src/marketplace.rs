//! FangHub marketplace client — install skills from the registry.
//!
//! For Phase 1, uses GitHub releases as the registry backend.
//! Each skill is a GitHub repo with releases containing the skill bundle.

use crate::SkillError;
use std::io::Cursor;
use std::path::{Component, Path, PathBuf};
use tracing::info;

/// FangHub registry configuration.
#[derive(Debug, Clone)]
pub struct MarketplaceConfig {
    /// Base URL for the registry API.
    pub registry_url: String,
    /// GitHub organization for community skills.
    pub github_org: String,
}

impl Default for MarketplaceConfig {
    fn default() -> Self {
        Self {
            registry_url: "https://api.github.com".to_string(),
            github_org: "openfang-skills".to_string(),
        }
    }
}

/// Client for the FangHub marketplace.
pub struct MarketplaceClient {
    config: MarketplaceConfig,
    http: reqwest::Client,
}

impl MarketplaceClient {
    /// Create a new marketplace client.
    pub fn new(config: MarketplaceConfig) -> Self {
        let mut headers = reqwest::header::HeaderMap::new();
        if let Ok(token) = std::env::var("GITHUB_TOKEN") {
            if let Ok(auth_value) =
                reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token))
            {
                headers.insert(reqwest::header::AUTHORIZATION, auth_value);
            }
        }

        Self {
            config,
            http: reqwest::Client::builder()
                .user_agent("openfang-skills/0.1")
                .default_headers(headers)
                .build()
                .expect("Failed to build HTTP client"),
        }
    }

    /// Search for skills by query string.
    pub async fn search(&self, query: &str) -> Result<Vec<SkillSearchResult>, SkillError> {
        let url = format!(
            "{}/search/repositories?q={}+org:{}&sort=stars",
            self.config.registry_url, query, self.config.github_org
        );

        let resp = self
            .http
            .get(&url)
            .header("Accept", "application/vnd.github.v3+json")
            .send()
            .await
            .map_err(|e| SkillError::Network(format!("Search request failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(SkillError::Network(format!(
                "Search returned status {}",
                resp.status()
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| SkillError::Network(format!("Parse search response: {e}")))?;

        let results = body["items"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .map(|item| SkillSearchResult {
                        name: item["name"].as_str().unwrap_or("").to_string(),
                        description: item["description"].as_str().unwrap_or("").to_string(),
                        stars: item["stargazers_count"].as_u64().unwrap_or(0),
                        url: item["html_url"].as_str().unwrap_or("").to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(results)
    }

    /// Install a skill from a GitHub repo or a direct ZIP URL.
    ///
    /// If `source` is "owner/repo@skill", it fetches from that repo.
    /// If `source` is a simple name, it defaults to the marketplace organization.
    pub async fn install(&self, source: &str, target_dir: &Path) -> Result<String, SkillError> {
        let (repo, skill_name) = if source.contains('/') {
            // Handle "owner/repo" or "owner/repo@skill_name"
            let parts: Vec<&str> = source.split('@').collect();
            let repo_full = parts[0];
            let name = parts.get(1).map(|s| s.to_string()).unwrap_or_else(|| {
                repo_full
                    .split('/')
                    .next_back()
                    .unwrap_or(repo_full)
                    .to_string()
            });
            (repo_full.to_string(), name)
        } else {
            // Default to marketplace org
            (format!("{}/{}", self.config.github_org, source), source.to_string())
        };

        let url = if repo.starts_with("http") {
             repo.clone() // Direct URL provided
        } else {
            format!(
                "{}/repos/{}/releases/latest",
                self.config.registry_url, repo
            )
        };

        info!("Fetching skill from {url}");

        let resp = self
            .http
            .get(&url)
            .header("Accept", "application/vnd.github.v3+json")
            .send()
            .await
            .map_err(|e| SkillError::Network(format!("Fetch source: {e}")))?;

        if !resp.status().is_success() {
            return Err(SkillError::NotFound(format!(
                "Source '{source}' not reachable (status {})",
                resp.status()
            )));
        }

        // If it's a GitHub Release API response
        let (version, archive_url) = if url.contains("/repos/") && url.contains("/releases/") {
            let release: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| SkillError::Network(format!("Parse release: {e}")))?;

            let v = release["tag_name"]
                .as_str()
                .unwrap_or("unknown")
                .to_string();

            let archive_url = release["zipball_url"]
                .as_str()
                .ok_or_else(|| SkillError::Network("No zipball URL in release".to_string()))?
                .to_string();
            (v, archive_url)
        } else {
            // Direct URL logic: currently only ZIP archives are supported.
            if !url.to_ascii_lowercase().ends_with(".zip") {
                return Err(SkillError::RuntimeNotAvailable(
                    "Only ZIP archives are currently supported for direct URL skill installs."
                        .to_string(),
                ));
            }
            ("latest".to_string(), url)
        };

        info!("Downloading skill {skill_name} {version} from {archive_url}...");

        let skill_dir = target_dir.join(&skill_name);
        if skill_dir.exists() {
            return Err(SkillError::AlreadyInstalled(skill_name));
        }
        std::fs::create_dir_all(&skill_dir)?;

        // Download the archive bytes.
        let archive_resp = self
            .http
            .get(&archive_url)
            .send()
            .await
                .map_err(|e| SkillError::Network(format!("Download failed: {e}")))?;

        if !archive_resp.status().is_success() {
            return Err(SkillError::Network(format!(
                "Download status: {}",
                archive_resp.status()
            )));
        }

        let archive_bytes = archive_resp
            .bytes()
            .await
            .map_err(|e| SkillError::Network(format!("Read archive bytes: {e}")))?;

        extract_zip_archive(&archive_bytes, &skill_dir)?;

        let manifest_path = skill_dir.join("skill.toml");
        let skillmd_path = skill_dir.join("SKILL.md");
        let package_json_path = skill_dir.join("package.json");
        if !(manifest_path.exists() || skillmd_path.exists() || package_json_path.exists()) {
            let _ = std::fs::remove_dir_all(&skill_dir);
            return Err(SkillError::InvalidManifest(
                "Archive did not contain skill.toml, SKILL.md, or package.json at the skill root."
                    .to_string(),
            ));
        }

        // Metadata for registry tracking
        let meta = serde_json::json!({
            "name": skill_name,
            "version": version,
            "source": archive_url,
            "installed_at": chrono::Utc::now().to_rfc3339(),
        });
        std::fs::write(
            skill_dir.join("marketplace_meta.json"),
            serde_json::to_string_pretty(&meta).unwrap_or_default(),
        )?;

        info!("Successfully fetched skill: {skill_name} to {}", skill_dir.display());
        Ok(version)
    }
}

fn extract_zip_archive(bytes: &[u8], skill_dir: &Path) -> Result<(), SkillError> {
    let reader = Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(reader)
        .map_err(|e| SkillError::InvalidManifest(format!("Open ZIP archive: {e}")))?;
    let common_root = common_top_level_dir(&mut archive);

    let mut extracted_any = false;
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| SkillError::InvalidManifest(format!("Read ZIP entry: {e}")))?;
        let entry_path = file
            .enclosed_name()
            .ok_or_else(|| SkillError::InvalidManifest("ZIP contains invalid path".to_string()))?;

        let relative = strip_common_root(&entry_path, common_root.as_deref());
        if relative.as_os_str().is_empty() {
            continue;
        }

        let out_path = skill_dir.join(relative);
        if file.is_dir() {
            std::fs::create_dir_all(&out_path)?;
            continue;
        }

        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&out_path)?;
        std::io::copy(&mut file, &mut out)?;
        extracted_any = true;
    }

    if !extracted_any {
        return Err(SkillError::InvalidManifest(
            "ZIP archive did not contain any extractable files.".to_string(),
        ));
    }

    Ok(())
}

fn common_top_level_dir<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> Option<PathBuf> {
    let mut root: Option<PathBuf> = None;
    for i in 0..archive.len() {
        let file = archive.by_index(i).ok()?;
        let path = file.enclosed_name()?;
        let mut components = path.components();
        let first = match components.next() {
            Some(Component::Normal(part)) => PathBuf::from(part),
            _ => continue,
        };
        match &root {
            Some(existing) if existing != &first => return None,
            Some(_) => {}
            None => root = Some(first),
        }
    }
    root
}

fn strip_common_root(path: &Path, common_root: Option<&Path>) -> PathBuf {
    let mut components = path.components();
    if common_root.is_some() {
        let _ = components.next();
    }
    let stripped: PathBuf = components
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part),
            _ => None,
        })
        .collect();
    if stripped.as_os_str().is_empty() {
        path.file_name().map(PathBuf::from).unwrap_or_default()
    } else {
        stripped
    }
}

/// A search result from the marketplace.
#[derive(Debug, Clone)]
pub struct SkillSearchResult {
    /// Skill name.
    pub name: String,
    /// Description.
    pub description: String,
    /// Star count.
    pub stars: u64,
    /// Repository URL.
    pub url: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = MarketplaceConfig::default();
        assert!(config.registry_url.contains("github"));
        assert_eq!(config.github_org, "openfang-skills");
    }

    #[test]
    fn test_client_creation() {
        let client = MarketplaceClient::new(MarketplaceConfig::default());
        assert_eq!(client.config.github_org, "openfang-skills");
    }
}
