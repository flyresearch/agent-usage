use std::path::PathBuf;

// Native qualification can relocate a cross-built test bundle. This override
// belongs only to tests; production never reads it or discovers executables.
pub fn binary() -> PathBuf {
    let path = std::env::var_os("AGENT_USAGE_TEST_BINARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_agent-usage")));
    assert!(
        path.is_absolute() && path.is_file(),
        "test collector is missing"
    );
    path
}
