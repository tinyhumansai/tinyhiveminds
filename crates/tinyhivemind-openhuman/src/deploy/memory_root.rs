//! Logical root identities encode injectively into native namespace segments.
use super::{DeployResult, runtime::unsupported};
/// Encode a logical root as a stable native namespace below `project:hivemind`.
/// Every UTF-8 byte becomes two hex digits, grouped into supported `team` segments.
/// All roots use this encoding, so a logical root resembling an encoded root cannot collide.
/// # Errors
/// Returns `UnsupportedSetting` for blank roots or roots exceeding 384 UTF-8 bytes,
/// the native root's seven-segment bound, reserving one segment for each agent.
pub fn native_memory_root(logical: &str) -> DeployResult<String> {
    if logical.trim().is_empty() || logical.len() > 384 {
        return Err(unsupported("memory root exceeds native namespace bounds"));
    }
    let mut root = String::from("project:hivemind");
    for chunk in logical.as_bytes().chunks(64) {
        root.push_str("/team:");
        root.extend(chunk.iter().map(|byte| format!("{byte:02x}")));
    }
    Ok(root)
}
