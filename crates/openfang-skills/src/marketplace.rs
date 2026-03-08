//! FangHub marketplace client — install skills from the registry.
//!
//! For Phase 1, uses GitHub releases as the registry backend.
//! Each skill is a GitHub repo with releases containing the skill bundle.

use crate::SkillError;
use std::path::Path;
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
        Self {
            config,
            http: reqwest::Client::builder()
                .user_agent("openfang-skills/0.1")
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

    /// Install a skill from a GitHub repo or a direct ZIP/TAR URL.
    ///
    /// If `source` is "owner/repo@skill", it fetches from that repo.
    /// If `source` is a simple name, it defaults to the marketplace organization.
    pub async fn install(&self, source: &str, target_dir: &Path) -> Result<String, SkillError> {
        let (repo, skill_name) = if source.contains('/') {
            // Handle "owner/repo" or "owner/repo@skill_name"
            let parts: Vec<&str> = source.split('@').collect();
            let repo_full = parts[0];
            let name = parts.get(1).map(|s| s.to_string()).unwrap_or_else(|| {
                repo_full.split('/').last().unwrap_or(repo_full).to_string()
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
        let (version, tarball_url) = if url.contains("/repos/") && url.contains("/releases/") {
            let release: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| SkillError::Network(format!("Parse release: {e}")))?;

            let v = release["tag_name"]
                .as_str()
                .unwrap_or("unknown")
                .to_string();

            let t_url = release["tarball_url"]
                .as_str()
                .ok_or_else(|| SkillError::Network("No tarball URL in release".to_string()))?
                .to_string();
            (v, t_url)
        } else {
            // Direct URL logic
            ("latest".to_string(), url)
        };

        info!("Downloading skill {skill_name} {version} from {tarball_url}...");

        let skill_dir = target_dir.join(&skill_name);
        if !skill_dir.exists() {
            std::fs::create_dir_all(&skill_dir)?;
        }

        // Download the binary stream
        let tar_resp = self
            .http
            .get(&tarball_url)
            .send()
            .await
            .map_err(|e| SkillError::Network(format!("Download failed: {e}")))?;

        if !tar_resp.status().is_success() {
            return Err(SkillError::Network(format!(
                "Download status: {}",
                tar_resp.status()
            )));
        }

        // Metadata for registry tracking
        let meta = serde_json::json!({
            "name": skill_name,
            "version": version,
            "source": tarball_url,
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
