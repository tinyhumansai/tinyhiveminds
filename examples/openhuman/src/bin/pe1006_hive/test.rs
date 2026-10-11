//! Turn-budget regression tests for the PE1006 live runner.

use super::ensure_round_fits;

#[test]
fn refuses_to_launch_a_round_past_the_exact_turn_cap() {
    ensure_round_fits(21, 4).expect("round ending exactly at the cap");
    let error = ensure_round_fits(22, 4).expect_err("oversized round rejected");
    assert!(error.to_string().contains("exceed 25 turns"));
}

#[test]
fn memory_bootstrap_selects_only_harness_initialization_and_legacy_queue() {
    let services = super::memory_services();
    assert!(services.harness_init && services.memory_queue);
    assert!(!services.rpc_http && !services.socketio && !services.cron && !services.channels);
    assert!(!services.login_gated && !services.update_scheduler);
    assert!(!services.skill_catalog_refresh && !services.mcp_boot);
    assert!(!services.integrations && !services.memory_sync);
}
