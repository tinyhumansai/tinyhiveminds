//! Failed provider calls retain diagnostic evidence without credentials.
use super::{Provider, Result};
#[test]
fn saves_failure_response_and_bounds_redacted_diagnostics() -> Result<()> {
    let output = std::path::PathBuf::from("target/live-provider-failure-regression")
        .join(std::process::id().to_string());
    std::fs::create_dir_all(&output)?;
    let mut provider = Provider {
        key: "private-test-key".into(),
        model: "fixture".into(),
        output: output.clone(),
        calls: 1,
        tokens: 0,
    };
    let body = b"private-test-key provider failure";
    let result = provider.decode_response(
        false,
        "exit status: 22",
        body,
        format!("private-test-key transport failed {}", "x".repeat(1000)).as_bytes(),
    );
    let error = result.err().ok_or("expected provider failure")?.to_string();
    let saved = std::fs::read_to_string(output.join("response-1.error"))?;
    assert!(saved.contains("provider failure"));
    assert!(!saved.contains("private-test-key"));
    assert!(error.contains("transport failed"));
    assert!(error.len() < 400);
    assert!(!error.contains("private-test-key"));
    std::fs::remove_dir_all(output)?;
    Ok(())
}
