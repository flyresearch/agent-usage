use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const PARSER_VERSION: &str = "p2a-5";
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Row {
    pub source_id: String,
    pub agent: String,
    pub model: String,
    pub day: Option<String>,
    pub time_basis: String,
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
