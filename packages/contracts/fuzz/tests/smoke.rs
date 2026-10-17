//! Deterministic smoke run of the fuzz harness on stable Rust, so the harness is
//! known to work without a nightly toolchain: `cargo test` inside `fuzz/`.
use arbitrary::{Arbitrary, Unstructured};
use backit_fuzz::{run_market, MarketInput};

#[test]
fn harness_runs_cleanly_on_pseudo_random_inputs() {
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut buf = [0u8; 512];
    for _ in 0..200 {
        for byte in buf.iter_mut() {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            *byte = (state >> 56) as u8;
        }
        if let Ok(input) = MarketInput::arbitrary(&mut Unstructured::new(&buf)) {
            run_market(input);
        }
    }
}

#[test]
fn boundary_amounts_are_rejected_without_crashing() {
    use backit_fuzz::{Amount, StakeOp};
    run_market(MarketInput {
        outcome_count: 32,
        creator_stake: Amount::MinStake,
        stakes: vec![
            StakeOp { staker: 0, position: 1, amount: Amount::Max },
            StakeOp { staker: 0, position: 1, amount: Amount::Stroop },
            StakeOp { staker: 0, position: 33, amount: Amount::MinStake },
            StakeOp { staker: 1, position: 0, amount: Amount::MinStake },
            StakeOp { staker: 1, position: 32, amount: Amount::MinStake },
            StakeOp { staker: 1, position: 32, amount: Amount::Max },
        ],
    });
    // 33 outcomes and 1 outcome must both be refused cleanly.
    for count in [0u8, 1, 33, 255] {
        run_market(MarketInput { outcome_count: count, creator_stake: Amount::MinStake, stakes: vec![] });
    }
}
