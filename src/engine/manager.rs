use super::matcher::HashedMatcher;
use super::traits::{BlocklistManager, BlocklistMatcher};
use crate::config::Config;
use futures::{stream, StreamExt};
use reqwest::Client;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{error, info};

pub struct StandardManager {
    config: Config,
    client: Client,
}

impl StandardManager {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            client: Client::builder().user_agent("AdNope/1.0").build().unwrap(),
        }
    }

    fn parse_blocklist_content(text: &str, source_id: u8) -> Vec<(String, u8)> {
        let mut entries = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            // Skip comments and empty lines
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // Simple domain list format: one domain per line
            entries.push((line.to_string(), source_id));
        }
        entries
    }

    async fn fetch_and_parse(client: &Client, url: String, source_id: u8) -> Vec<(String, u8)> {
        info!("Fetching blocklist [{}] from {}", source_id, url);
        match client.get(&url).send().await {
            Ok(resp) => match resp.text().await {
                Ok(text) => {
                    let entries = Self::parse_blocklist_content(&text, source_id);
                    info!("Parsed {} entries from source {}", entries.len(), source_id);
                    entries
                }
                Err(e) => {
                    error!("Failed to read body from {}: {}", url, e);
                    vec![]
                }
            },
            Err(e) => {
                error!("Failed to fetch {}: {}", url, e);
                vec![]
            }
        }
    }
}

#[async_trait::async_trait]
impl BlocklistManager for StandardManager {
    async fn refresh(&self) -> Arc<dyn BlocklistMatcher> {
        info!("Refreshing blocklists...");

        let client = self.client.clone();
        let urls = self.config.blocklists.clone();

        let tasks = urls.into_iter().enumerate().map(|(idx, url)| {
            let client = client.clone();
            let source_id = if idx > 255 { 255 } else { idx as u8 };
            async move { Self::fetch_and_parse(&client, url, source_id).await }
        });

        let results: Vec<Vec<(String, u8)>> = stream::iter(tasks)
            .buffer_unordered(self.config.updates.concurrent_downloads)
            .collect()
            .await;

        let mut map = HashMap::new();
        let mut total_count = 0;

        for list in results {
            for (domain, source) in list {
                map.insert(domain, source);
                total_count += 1;
            }
        }

        info!(
            "Blocklist refresh complete. Total distinct domains: {} (from {} raw entries)",
            map.len(),
            total_count
        );

        Arc::new(HashedMatcher::new(map, self.config.allowlist.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_blocklist_simple_format() {
        let content = "
        # Check comments
        example.com
        adserver.net
        # Empty line

        justadomain.com
        ";

        let entries = StandardManager::parse_blocklist_content(content, 1);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0], ("example.com".to_string(), 1));
        assert_eq!(entries[1], ("adserver.net".to_string(), 1));
        assert_eq!(entries[2], ("justadomain.com".to_string(), 1));
    }
}
