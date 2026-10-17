//! Property-based conservation tests (issue #699).
//!
//! Each case generates a random call (2-8 outcomes, 0-10% fee), a random
//! sequence of stakes from up to 16 stakers, and a random resolution. The
//! reference model then settles it and the tests assert the zero-loss
//! invariants:
//!
//! * the sum of per-staker stakes equals the pool (the vault balance);
//! * when the winning outcome has backers, payouts + fees account for the vault
//!   up to integer-division dust (at most 2 units per winner), every winner gets
//!   at least their stake back, and the fee collected never exceeds its cap;
//! * when nobody backed the winning outcome, or the call is voided, every staker
//!   can be refunded in full and the refunds sum to the vault exactly, so no
//!   collateral is locked.
//!
//! 50,000 cases run by default. For a quick local run:
//!   PROPTEST_CASES=500 cargo test -p invariant-tests --test conservation_proptest

use std::collections::BTreeMap;

use invariant_tests::model::{CallStatus, ModelState};
use proptest::prelude::*;

const CASES: u32 = 50_000;
const END_TS: u64 = 10_000;

#[derive(Debug, Clone)]
struct Scenario {
    fee_bps: u32,
    outcome_count: u32,
    /// (staker index, outcome, amount)
    stakes: Vec<(usize, u32, i128)>,
    winning_outcome: u32,
    void_instead: bool,
}

fn scenario() -> impl Strategy<Value = Scenario> {
    (0u32..=1_000, 2u32..=8).prop_flat_map(|(fee_bps, outcome_count)| {
        (
            Just(fee_bps),
            Just(outcome_count),
            prop::collection::vec((0usize..16, 1..=outcome_count, 1i128..=1_000_000_000_000), 0..60),
            1..=outcome_count,
            any::<bool>(),
        )
            .prop_map(
                |(fee_bps, outcome_count, stakes, winning_outcome, void_instead)| Scenario {
                    fee_bps,
                    outcome_count,
                    stakes,
                    winning_outcome,
                    void_instead,
                },
            )
    })
}

fn staker(i: usize) -> String {
    format!("staker_{i}")
}

proptest! {
    #![proptest_config(ProptestConfig { cases: CASES, ..ProptestConfig::default() })]

    #[test]
    fn funds_are_conserved_and_never_locked(s in scenario()) {
        let mut state = ModelState::new(s.fee_bps, 604_800);
        let call_id = state.create_call("creator", 1, END_TS, s.outcome_count);

        // Independent bookkeeping of what each staker put in.
        let mut per_staker: BTreeMap<String, i128> = BTreeMap::new();
        let mut pool: i128 = 0;
        for (who, outcome, amount) in &s.stakes {
            state.stake(call_id, &staker(*who), *amount, *outcome);
            *per_staker.entry(staker(*who)).or_default() += *amount;
            pool += *amount;
        }

        // Invariant 1: sum of user stakes == the call's pool == the vault.
        let model_pool = state.calls[&call_id].total_stake();
        prop_assert_eq!(per_staker.values().sum::<i128>(), pool);
        prop_assert_eq!(model_pool, pool);

        let winners_exist = state.calls[&call_id]
            .outcome_stakes
            .get(&s.winning_outcome)
            .copied()
            .unwrap_or(0)
            > 0;

        if s.void_instead || !winners_exist {
            // Refund path: everything is returned, nothing is locked.
            state.void_call(call_id);
            let mut refunded: i128 = 0;
            for who in per_staker.keys() {
                refunded += state.claim_void_refund(call_id, who);
            }
            prop_assert_eq!(refunded, pool, "refunds must equal the vault exactly");
            return Ok(());
        }

        state.resolve_call(call_id, s.winning_outcome, END_TS);
        state.mark_settled(call_id);
        let settled = matches!(state.calls[&call_id].status, CallStatus::Settled { .. });
        prop_assert!(settled, "the call must be settled");

        let winning_pool = state.calls[&call_id].outcome_stakes[&s.winning_outcome];
        let losing_pool = pool - winning_pool;
        let fee_cap = losing_pool * s.fee_bps as i128 / 10_000;

        let mut winners = 0i128;
        let (mut paid, mut fees) = (0i128, 0i128);
        for who in per_staker.keys() {
            let stake = state.calls[&call_id].staker_stake(s.winning_outcome, who);
            if stake == 0 {
                continue;
            }
            winners += 1;
            let (fee_share, payout) = state.claim_payout(call_id, who);
            prop_assert!(payout >= stake, "a winner never gets back less than their stake");
            prop_assert!(fee_share >= 0);
            paid += payout;
            fees += fee_share;
        }

        // Invariant 2: payouts + fees account for the vault, up to rounding dust.
        prop_assert!(fees <= fee_cap, "fees {fees} exceed the cap {fee_cap}");
        prop_assert!(paid + fees <= pool, "cannot distribute more than the vault holds");
        let dust = pool - (paid + fees);
        prop_assert!(
            dust <= 2 * winners,
            "dust {dust} exceeds 2 per winner ({winners} winners)"
        );
    }
}
