//! Target triples and version numbers.

use std::cmp::Ordering;

/// The triple this `whet` was built for (overridable with `WHET_TARGET`, e.g. to cross-install).
pub fn target() -> String {
    match std::env::var("WHET_TARGET") {
        Ok(t) if !t.trim().is_empty() => t.trim().to_string(),
        _ => env!("WHET_TARGET").to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub nums: [u64; 3],
    pub pre: Option<String>,
}

impl Version {
    /// `1.2.3` or `1.2.3-rc.1`.
    pub fn parse(s: &str) -> Option<Version> {
        let (core, pre) = match s.split_once('-') {
            Some((c, p)) => (c, Some(p.to_string())),
            None => (s, None),
        };
        if let Some(p) = &pre {
            if p.is_empty() || !p.chars().all(|c| c.is_ascii_alphanumeric() || c == '.') {
                return None;
            }
        }
        let parts: Vec<&str> = core.split('.').collect();
        if parts.len() != 3 {
            return None;
        }
        let mut nums = [0u64; 3];
        for (i, p) in parts.iter().enumerate() {
            if p.is_empty() || !p.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            nums[i] = p.parse().ok()?;
        }
        Some(Version { nums, pre })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.nums.cmp(&other.nums).then_with(|| match (&self.pre, &other.pre) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Greater, // 1.0.0 > 1.0.0-rc.1
            (Some(_), None) => Ordering::Less,
            (Some(a), Some(b)) => a.cmp(b),
        })
    }
}

pub fn valid_version(s: &str) -> bool {
    Version::parse(s).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_orders_versions() {
        let v = |s| Version::parse(s).unwrap();
        assert!(v("0.2.10") > v("0.2.9"));
        assert!(v("1.0.0") > v("0.99.99"));
        assert!(v("1.0.0") > v("1.0.0-rc.1"));
        assert!(v("1.0.0-rc.2") > v("1.0.0-rc.1"));
        assert_eq!(v("1.2.3"), v("1.2.3"));
        for bad in ["", "1", "1.2", "1.2.x", "1.2.3.4", "v1.2.3", "../../etc", "1.2.3-", "1.2.3-a/b"] {
            assert!(Version::parse(bad).is_none(), "{} should be rejected", bad);
        }
    }

    #[test]
    fn target_has_a_triple_shape() {
        let t = target();
        assert!(t.split('-').count() >= 3, "{}", t);
    }
}
