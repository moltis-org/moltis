//! `tools.agent_max_cost_usd` enforcement.

use crate::{model::Usage, runner::RunCostGate};

fn usage(input: u32, output: u32) -> Usage {
    Usage {
        input_tokens: input,
        output_tokens: output,
        ..Default::default()
    }
}

#[test]
fn no_cap_means_no_gate() {
    let gate = RunCostGate::from_limit(None, "local-model-without-a-price").unwrap();
    assert!(gate.is_none());
}

#[test]
fn gate_refuses_next_call_once_cap_is_spent() {
    // gpt-4o: $2.50 / 1M input, $10 / 1M output.
    let gate = RunCostGate::from_limit(Some(0.05), "gpt-4o")
        .unwrap()
        .expect("cap set");
    gate.check_before_call().expect("fresh run is under cap");

    // 10k input + 1k output = $0.025 + $0.01 = $0.035: still under $0.05.
    gate.record(&usage(10_000, 1_000)).unwrap();
    assert!(
        (gate.spent_usd() - 0.035).abs() < 1e-9,
        "spent {}",
        gate.spent_usd()
    );
    gate.check_before_call().expect("still under cap");

    // A second request of the same size crosses the cap; the next provider
    // call must be refused even though the iteration cap would allow it.
    gate.record(&usage(10_000, 1_000)).unwrap();
    let err = gate.check_before_call().unwrap_err().to_string();
    assert!(
        err.contains("exceeded cost budget"),
        "unexpected error: {err}"
    );
}

#[test]
fn cache_tokens_are_priced_separately_from_fresh_input() {
    let gate = RunCostGate::from_limit(Some(10.0), "gpt-4o")
        .unwrap()
        .expect("cap set");
    // moltis `Usage` keeps fresh input, cache reads and cache writes in
    // separate buckets; cache reads must not be billed at the full rate.
    gate.record(&Usage {
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 1_000_000,
        cache_write_tokens: 0,
    })
    .unwrap();
    assert!(
        gate.spent_usd() < 2.5,
        "cache reads billed at full input price: {}",
        gate.spent_usd()
    );
    assert!(gate.spent_usd() > 0.0);
}

#[test]
fn unpriced_model_is_refused_instead_of_running_uncapped() {
    let err = RunCostGate::from_limit(Some(1.0), "definitely-not-a-real-model-xyz")
        .err()
        .expect("unpriced model must be refused")
        .to_string();
    assert!(err.contains("no known price"), "unexpected error: {err}");
}

#[test]
fn non_positive_cap_is_rejected() {
    assert!(RunCostGate::from_limit(Some(0.0), "gpt-4o").is_err());
    assert!(RunCostGate::from_limit(Some(f64::NAN), "gpt-4o").is_err());
}
