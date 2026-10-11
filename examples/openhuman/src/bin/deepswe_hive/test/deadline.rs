//! Native cancellation RPC errors keep fail-closed deadline accounting.
use super::super::{CoreError, deadline_result};
use std::time::Duration;

#[test]
fn cancellation_rpc_at_the_selected_deadline_is_a_timeout() {
    let error = CoreError::Rpc {
        method: "inference.agent_chat",
        message: "fixture cancellation".into(),
    };
    assert!(
        deadline_result::<()>(Err(error), Duration::from_secs(2), Duration::from_secs(2)).is_err()
    );
}

#[test]
fn an_early_rpc_failure_keeps_provider_failure_accounting() {
    let error = CoreError::Rpc {
        method: "inference.agent_chat",
        message: "fixture provider failure".into(),
    };
    assert!(matches!(
        deadline_result::<()>(Err(error), Duration::from_millis(1), Duration::from_secs(2)),
        Ok(Err(CoreError::Rpc { .. }))
    ));
}

#[test]
fn native_deadline_error_is_a_timeout_even_before_clock_accounting() {
    let error = CoreError::DeadlineExceeded {
        method: "inference.agent_chat",
    };
    assert!(deadline_result::<()>(Err(error), Duration::ZERO, Duration::from_secs(2)).is_err());
}
