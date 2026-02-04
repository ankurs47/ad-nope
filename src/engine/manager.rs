use super::matcher::HashedMatcher;
use super::traits::{BlocklistManager, BlocklistMatcher};
use crate::config::Config;
use futures::{stream, StreamExt};
use reqwest::Client;
use rustc_hash::FxHashMap;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_util::io::StreamReader;
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

    fn parse_line(line: &str) -> Option<Box<str>> {
        let line = line.trim();
        // Skip comments and empty lines
        if line.is_empty() || line.starts_with('#') {
            return None;
        }
        // Simple domain list format: one domain per line
        // Normalize to lowercase
        Some(line.to_lowercase().into_boxed_str())
    }

    #[cfg(test)]
    fn parse_blocklist_content(text: &str, source_id: u8) -> Vec<(Box<str>, u8)> {
        text.lines()
            .filter_map(|line| Self::parse_line(line).map(|d| (d, source_id)))
            .collect()
    }

    async fn fetch_and_parse(
        client: &Client,
        name: String,
        url: String,
        source_id: u8,
    ) -> Vec<(Box<str>, u8)> {
        info!(
            "Fetching blocklist '{}' (ID {}) from {}",
            name, source_id, url
        );
        match client.get(&url).send().await {
            Ok(resp) => {
                let stream = resp
                    .bytes_stream()
                    .map(|result| result.map_err(std::io::Error::other));
                let reader = StreamReader::new(stream);
                let mut lines = BufReader::new(reader).lines();
                let mut entries = Vec::new();

                while let Ok(Some(line)) = lines.next_line().await {
                    if let Some(domain) = Self::parse_line(&line) {
                        entries.push((domain, source_id));
                    }
                }

                info!(
                    "Parsed {} entries from '{}' (ID {})",
                    entries.len(),
                    name,
                    source_id
                );
                entries
            }
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
        // Use sorted list to ensure deterministic source IDs
        let blocklists = self.config.get_blocklists_sorted();

        let tasks = blocklists
            .into_iter()
            .enumerate()
            .map(|(idx, (name, url))| {
                let client = client.clone();
                // Source ID matches the index in the sorted list (0-255)
                let source_id = if idx > 255 { 255 } else { idx as u8 };
                async move { Self::fetch_and_parse(&client, name, url.clone(), source_id).await }
            });

        let results: Vec<Vec<(Box<str>, u8)>> = stream::iter(tasks)
            .buffer_unordered(self.config.updates.concurrent_downloads)
            .collect()
            .await;

        let mut map = FxHashMap::default();
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
        assert_eq!(entries[0], ("example.com".to_string().into_boxed_str(), 1));
        assert_eq!(entries[1], ("adserver.net".to_string().into_boxed_str(), 1));
        assert_eq!(
            entries[2],
            ("justadomain.com".to_string().into_boxed_str(), 1)
        );
    }
}
