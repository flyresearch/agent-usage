use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const PARSER_VERSION: &str = "p2b-12";
pub type Result<T> = std::result::Result<T, Error>;

// Static error codes only: never expose a serde/io error or source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Error(pub &'static str);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Error {}

#[derive(Debug, Clone)]
pub struct CollectionFailure {
    pub error: Error,
    pub source_id: Option<String>,
}
impl From<Error> for CollectionFailure {
    fn from(error: Error) -> Self {
        Self {
            error,
            source_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema_version: u32,
    pub host_id: String,
    pub sources: Vec<Source>,
    #[serde(default)]
    pub model_aliases: BTreeMap<String, String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "policy_present"
    )]
    pub model_name_policy: Option<ModelNamePolicy>,
}
fn policy_present<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<ModelNamePolicy>, D::Error> {
    ModelNamePolicy::deserialize(d).map(Some)
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelNamePolicy {
    pub mode: String,
    #[serde(deserialize_with = "unique_sources")]
    pub sources: BTreeMap<String, Vec<String>>,
}
fn unique_sources<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<BTreeMap<String, Vec<String>>, D::Error> {
    use serde::de::{Error as _, MapAccess, Visitor};
    struct V;
    impl<'de> Visitor<'de> for V {
        type Value = BTreeMap<String, Vec<String>>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("unique sources")
        }
        fn visit_map<M: MapAccess<'de>>(
            self,
            mut m: M,
        ) -> std::result::Result<Self::Value, M::Error> {
            let mut out = BTreeMap::new();
            while let Some((k, v)) = m.next_entry()? {
                if out.insert(k, v).is_some() {
                    return Err(M::Error::custom("duplicate source"));
                }
            }
            Ok(out)
        }
    }
    d.deserialize_map(V)
}
impl Config {
    pub fn validate_policy(&self) -> Result<()> {
        if let Some(p) = &self.model_name_policy {
            if p.mode != "pattern" || p.sources.is_empty() || p.sources.len() > 64 {
                return Err(Error("invalid_config"));
            }
            for (id, prefixes) in &p.sources {
                if !self.sources.iter().any(|s| &s.id == id)
                    || prefixes.is_empty()
                    || prefixes.len() > 32
                {
                    return Err(Error("invalid_config"));
                }
                let mut seen = std::collections::BTreeSet::new();
                for prefix in prefixes {
                    let b = prefix.as_bytes();
                    if b.is_empty()
                        || b.len() > 48
                        || !b[0].is_ascii_alphabetic()
                        || b.last() != Some(&b'-')
                        || !label(prefix)
                        || !seen.insert(prefix)
                    {
                        return Err(Error("invalid_config"));
                    }
                }
            }
        }
        Ok(())
    }
    pub fn resolver<'a>(&'a self, source: &str) -> ModelResolver<'a> {
        ModelResolver {
            aliases: &self.model_aliases,
            prefixes: self
                .model_name_policy
                .as_ref()
                .and_then(|p| p.sources.get(source))
                .map(Vec::as_slice)
                .unwrap_or(&[]),
        }
    }
}
pub struct ModelResolver<'a> {
    pub aliases: &'a BTreeMap<String, String>,
    pub prefixes: &'a [String],
}
impl ModelResolver<'_> {
    pub fn automatic(&self, s: &str) -> bool {
        s != "unknown"
            && label(s)
            && self.prefixes.iter().any(|p| {
                s.strip_prefix(p)
                    .is_some_and(|tail| tail.as_bytes().first().is_some_and(u8::is_ascii_digit))
            })
    }
    pub fn get<'a>(&'a self, raw: &'a str) -> Option<&'a str> {
        self.aliases
            .get(raw)
            .map(String::as_str)
            .or_else(|| self.automatic(raw).then_some(raw))
    }
    pub fn permits(&self, name: &str) -> bool {
        name == "unknown" || self.aliases.values().any(|v| v == name) || self.automatic(name)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub id: String,
    pub adapter: String,
    pub paths: Vec<PathBuf>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    pub input: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub output: u64,
    pub reasoning: u64,
}
impl Tokens {
    pub fn array(self) -> [u64; 5] {
        [
            self.input,
            self.cache_read,
            self.cache_write,
            self.output,
            self.reasoning,
        ]
    }
    pub fn from_array(a: [u64; 5]) -> Self {
        Self {
            input: a[0],
            cache_read: a[1],
            cache_write: a[2],
            output: a[3],
            reasoning: a[4],
        }
    }
    pub fn checked_add(self, other: Self) -> Result<Self> {
        let mut a = self.array();
        for (x, y) in a.iter_mut().zip(other.array()) {
            *x = x.checked_add(y).ok_or(Error("token_overflow"))?;
        }
        let result = Self::from_array(a);
        result.total()?;
        Ok(result)
    }
    pub fn total(self) -> Result<u64> {
        self.array()
            .into_iter()
            .try_fold(0u64, |a, b| a.checked_add(b).ok_or(Error("token_overflow")))
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Coverage {
    pub input_reclassifications: u64,
    pub deduplicated_symlink_files: u64,
    pub runtime_usage_records: u64,
    pub missing_backend_threads: u64,
    pub auxiliary_usage_records: u64,
    pub residual_usage_records: u64,
    pub negative_residual_records: u64,
    pub orphan_usage_records: u64,
    pub unclassified_usage_records: u64,
    pub missing_identity_records: u64,
    pub ambiguous_time_records: u64,
    pub invalid_cache_ttl_records: u64,
    pub unmetered_files: u64,
    pub unclassified_fork_prefix_records: u64,
    pub response_usage_records: u64,
    pub mirrored_usage_records: u64,
    pub context_estimate_records: u64,
    pub usage_records: u64,
    pub pending_files: u64,
    pub unknown_total_carry: u64,
    pub missing_time_records: u64,
    pub unknown_model_records: u64,
    pub unsupported_fields: u64,
    pub counter_resets: u64,
    pub streaming_regressions: u64,
    pub unallocated_tokens: u64,
}
/// Count of deduplicated assistant responses with metered usage, not turns or log lines.
/// Legacy snapshots and aggregate-only records have unknown coverage.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageCount {
    pub known: u64,
    pub complete: bool,
}
impl MessageCount {
    pub fn one() -> Self {
        Self {
            known: 1,
            complete: true,
        }
    }
    pub fn checked_add(self, other: Self) -> Result<Self> {
        Ok(Self {
            known: self
                .known
                .checked_add(other.known)
                .ok_or(Error("message_count_overflow"))?,
            complete: self.complete && other.complete,
        })
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageInterval {
    pub start: Option<String>,
    pub end: Option<String>,
}
impl UsageInterval {
    pub fn merge(&mut self, other: &Self) {
        self.start = self.start.as_ref().zip(other.start.as_ref()).map(|(a, b)| {
            if chrono::DateTime::parse_from_rfc3339(a).ok()
                <= chrono::DateTime::parse_from_rfc3339(b).ok()
            {
                a.clone()
            } else {
                b.clone()
            }
        });
        self.end = self.end.as_ref().zip(other.end.as_ref()).map(|(a, b)| {
            if chrono::DateTime::parse_from_rfc3339(a).ok()
                >= chrono::DateTime::parse_from_rfc3339(b).ok()
            {
                a.clone()
            } else {
                b.clone()
            }
        });
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Row {
    #[serde(default)]
    pub messages: MessageCount,
    pub source_id: String,
    pub agent: String,
    pub model: String,
    pub day: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hour: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_role: Option<String>,
    pub time_basis: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval: Option<UsageInterval>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    pub tokens: Tokens,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceStatus {
    pub source_id: String,
    pub status: String,
    pub files: u64,
    pub cache_hits: u64,
    pub bytes_read: u64,
    pub coverage: Coverage,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityBucket {
    pub timestamp: i64,
    pub agent: String,
    pub tokens: u64,
    pub messages: MessageCount,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentActivity {
    pub as_of: i64,
    pub window_seconds: u64,
    pub coverage_complete: bool,
    pub unlocated_records: u64,
    pub future_records: u64,
    pub unsupported_sources: Vec<String>,
    pub buckets: Vec<ActivityBucket>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Snapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recent_activity: Option<RecentActivity>,
    pub schema_version: u32,
    pub collector_version: String,
    pub parser_version: String,
    pub host_id: String,
    pub configuration_fingerprint: String,
    pub generated_at: String,
    pub timezone: String,
    pub complete: bool,
    pub coverage_complete: bool,
    pub sources: Vec<SourceStatus>,
    pub rows: Vec<Row>,
    pub totals: Tokens,
    pub cost: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sqlite_version: Option<String>,
}

pub fn label(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
}
pub fn hash(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}
