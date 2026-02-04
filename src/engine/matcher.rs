use super::traits::BlocklistMatcher;
use rustc_hash::FxHashMap;

/// Optimized in-memory matcher using FxHashMap<Box<str>, u8>
#[derive(Debug)]
pub struct HashedMatcher {
    // Map domain -> source_id
    domains: FxHashMap<Box<str>, u8>,
    allowlist: FxHashMap<Box<str>, ()>,
}

impl HashedMatcher {
    pub fn new(domains: FxHashMap<Box<str>, u8>, allowlist_vec: Vec<String>) -> Self {
        let mut allowlist = FxHashMap::default();
        for d in allowlist_vec {
            allowlist.insert(d.into_boxed_str(), ());
        }
        Self { domains, allowlist }
    }
}

impl BlocklistMatcher for HashedMatcher {
    fn check(&self, domain: &str) -> Option<u8> {
        // 1. Check Allowlist (Exact Match)
        if self.allowlist.contains_key(domain) {
            return None;
        }

        // 2. Iterative Suffix Match
        let mut part = domain;
        loop {
            if let Some(&source_id) = self.domains.get(part) {
                return Some(source_id);
            }

            // Strip leading label
            match part.find('.') {
                Some(idx) => {
                    part = &part[idx + 1..];
                    if part.is_empty() {
                        break;
                    }
                }
                None => break,
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_matcher_logic() {
        let mut map = FxHashMap::default();
        map.insert("example.com".to_string().into_boxed_str(), 1);
        map.insert("sub.ad.com".to_string().into_boxed_str(), 2);

        let allowlist = vec!["allowed.example.com".to_string()];

        let matcher = HashedMatcher::new(map, allowlist);

        // Exact match
        assert_eq!(matcher.check("example.com"), Some(1));

        // Suffix match (subdomain of blocked)
        assert_eq!(matcher.check("sub.example.com"), Some(1));
        assert_eq!(matcher.check("a.b.example.com"), Some(1));

        // Allowlist override
        assert_eq!(matcher.check("allowed.example.com"), None);

        // Exact match 2
        assert_eq!(matcher.check("sub.ad.com"), Some(2));
        assert_eq!(matcher.check("deep.sub.ad.com"), Some(2));

        // Unblocked
        assert_eq!(matcher.check("google.com"), None);
    }
}
