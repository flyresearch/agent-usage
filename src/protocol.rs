use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const PARSER_VERSION: &str = "p2b-5";
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
#[derive(Debug, Serialize, Deserialize)]
pub struct Snapshot {
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
