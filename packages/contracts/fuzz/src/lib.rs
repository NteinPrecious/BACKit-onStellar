//! Fuzz harness for the call registry's call creation and multi-outcome staking
//! (issue #700), driven by `fuzz_targets/market_settlement.rs`.
//!
//! The inputs deliberately concentrate on boundary values: outcome counts around
//! the 2..=32 limits, stakes of 1 stroop and `i128::MAX`, and positions just
//! outside the valid range. Every contract call goes through the `try_` client
//! so a rejected input must come back as an `Err`, never a crash, and after
//! each step the pools are checked against an independent tally of what was
//! accepted (conservation) and for negative values or overflow.
use arbitrary::Arbitrary;
use call_registry::types::{CallInitArgs, ConditionType};
use call_registry::{CallRegistry, CallRegistryClient};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{contract, contractimpl, Address, Bytes, BytesN, Env};

/// A token whose transfers always succeed and track balances, so the registry's
/// balance-delta accounting sees real movement.
#[contract]
pub struct FuzzToken;

#[contractimpl]
impl FuzzToken {
    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        let from_balance: i128 = env.storage().instance().get(&from).unwrap_or(0);
        env.storage().instance().set(&from, &from_balance.wrapping_sub(amount));
        let to_balance: i128 = env.storage().instance().get(&to).unwrap_or(0);
        env.storage().instance().set(&to, &to_balance.wrapping_add(amount));
    }

    pub fn balance(env: Env, id: Address) -> i128 {
        env.storage().instance().get(&id).unwrap_or(0)
    }
}

/// How a fuzzed amount is chosen. The named boundaries come up far more often
/// than they would from uniformly random `i128`s.
#[derive(Debug, Clone, Arbitrary)]
pub enum Amount {
    Stroop,
    MinStake,
    Max,
    MaxMinusOne,
    Zero,
    Negative(u32),
    Small(u32),
    Wide(i128),
}

impl Amount {
    pub fn value(&self) -> i128 {
        match self {
            Amount::Stroop => 1,
            Amount::MinStake => 1_000_000,
            Amount::Max => i128::MAX,
            Amount::MaxMinusOne => i128::MAX - 1,
            Amount::Zero => 0,
            Amount::Negative(n) => -(*n as i128) - 1,
            Amount::Small(n) => 1_000_000 + *n as i128,
            Amount::Wide(v) => *v,
        }
    }
}

#[derive(Debug, Clone, Arbitrary)]
pub struct StakeOp {
    pub staker: u8,
    /// Position byte; values outside 1..=outcome_count must be rejected.
    pub position: u8,
    pub amount: Amount,
}

#[derive(Debug, Clone, Arbitrary)]
pub struct MarketInput {
    /// Outcome count byte; only 2..=32 may create a call.
    pub outcome_count: u8,
    pub creator_stake: Amount,
    pub stakes: Vec<StakeOp>,
}

/// Run one fuzz case. Panics (the fuzzer's failure signal) only if an invariant
/// is violated or the host crashes; a contract rejecting an input is expected.
pub fn run_market(input: MarketInput) {
    let env = Env::default();
    env.mock_all_auths();
    env.cost_estimate().budget().reset_unlimited();
    env.ledger().set_timestamp(1_000);

    let admin = Address::generate(&env);
    let manager = Address::generate(&env);
    let creator = Address::generate(&env);
    let id = env.register(CallRegistry, ());
    let client = CallRegistryClient::new(&env, &id);
    client.initialize(&admin, &manager, &1_000_000);

    let token = env.register(FuzzToken, ());
    client.whitelist_token(&token);

    let outcome_count = input.outcome_count as u32;
    let args = CallInitArgs {
        stake_token: token,
        stake_amount: input.creator_stake.value(),
        start_price: 100_000_000,
        end_ts: 2_000,
        token_address: Address::generate(&env),
        pair_id: Bytes::from_slice(&env, b"USDC/XLM"),
        ipfs_cid: Bytes::from_slice(&env, b"cid"),
        metadata_hash: BytesN::from_array(&env, &[0u8; 32]),
        condition: ConditionType::TargetAbove(1),
        outcome_count,
        gate_kind: None,
        gate_min_account_age: 0,
        gate_min_xlm_balance: 0,
        gate_min_trustlines: 0,
        gate_badge: None,
    };

    let created = client.try_create_call(&creator, &args);
    let call = match created {
        Ok(Ok(call)) => call,
        // Rejected input: the only requirement is that it was a clean error.
        _ => {
            assert!(
                !(2..=32).contains(&outcome_count)
                    || input.creator_stake.value() < 1_000_000,
                "a valid call (count {outcome_count}, stake {}) was rejected",
                input.creator_stake.value()
            );
            return;
        }
    };
    assert!((2..=32).contains(&outcome_count), "created a call with {outcome_count} outcomes");

    let stakers: Vec<Address> = (0..8).map(|_| Address::generate(&env)).collect();
    let mut accepted = vec![0i128; outcome_count as usize];

    for op in input.stakes.iter().take(64) {
        let who = &stakers[op.staker as usize % stakers.len()];
        let amount = op.amount.value();
        let position = op.position as u32;

        if let Ok(Ok(updated)) = client.try_stake_on_call(who, &call.id, &amount, &position) {
            assert!(amount > 0, "a non-positive stake was accepted");
            assert!(
                (1..=outcome_count).contains(&position),
                "a stake on position {position} of {outcome_count} was accepted"
            );
            accepted[position as usize - 1] = accepted[position as usize - 1]
                .checked_add(amount)
                .expect("accepted stakes overflowed i128");
            let pool = updated.outcome_stakes.get(position).expect("pool exists");
            assert_eq!(pool, accepted[position as usize - 1], "pool must equal accepted stakes");
        }

        // Conservation and sanity after every step, accepted or not.
        let pools = client.get_outcome_stakes(&call.id);
        assert_eq!(pools.len(), outcome_count);
        let mut total: i128 = 0;
        for position in 1..=outcome_count {
            let pool = pools.get(position).expect("pool exists");
            assert!(pool >= 0, "negative pool");
            assert_eq!(pool, accepted[position as usize - 1], "pool {position} diverged from accepted stakes");
            total = total.checked_add(pool).expect("pools overflowed i128");
        }
        assert_eq!(total, accepted.iter().sum::<i128>());
    }
}
