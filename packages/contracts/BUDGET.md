# Contract Budget Reference

This workspace now enforces Soroban budget ceilings in tests and CI for the
highest-risk execution paths in `call_registry` and `outcome_manager`.

## Important Limitation

`packages/contracts` currently depends on `soroban-sdk 23.x`.

In this SDK line, `env.cost_estimate()` and `budget()` live behind
`testutils`, which means they are available in unit tests but are not
available inside production Wasm builds. Because of that:

- Production contracts cannot call `env.budget().reset(...)` directly.
- Production contracts cannot emit real `BudgetConsumed` telemetry events with
  live CPU or memory counters.
- Budget protection is enforced through optimized code paths plus CI tests that
  run each hot function under explicit Soroban limits.

## Enforced Targets

The following ceilings are enforced by the new `stays_within_budget` tests.
These numbers are intentionally conservative to leave headroom for routine
state growth while still catching regressions.

| Contract | Function | Scenario | CPU Limit | Memory Limit |
| --- | --- | --- | ---: | ---: |
| `call_registry` | `stake_on_call` | single stake on an active two-outcome call | 6,000,000 | 80,000 bytes |
| `call_registry` | `get_calls_paginated` | first page of 10 existing calls | 2,500,000 | 25,000 bytes |
| `call_registry` | `get_call_stakers` | capped read of 50 unique stakers | 2,500,000 | 25,000 bytes |
| `outcome_manager` | `claim_payout` | single winner claim with 5% fee | 3,500,000 | 45,000 bytes |
| `outcome_manager` | `batch_claim_payouts` | 20 winners in one batch with 5% fee | 20,000,000 | 180,000 bytes |

## Regression Coverage

The workspace now includes two explicit categories of budget tests:

- `stays_within_budget`: verifies the hot function completes within its
  assigned CPU and memory ceiling.
- `exceeding_budget_fails`: verifies Soroban rejects the invocation when the
  configured limit is intentionally too low.

CI runs both categories in `.github/workflows/contracts.yml`.

## Optimization Notes

The budget work also includes code-path optimizations:

- `call_registry::stake_on_call`
  - Removes redundant config and user-stake reads.
  - Uses membership keys to avoid O(n) duplicate scans in
    `add_call_staker` and `add_staker_call`.
- `call_registry::get_call_stakers`
  - Caps the default response to 50 addresses.
  - Adds `get_call_stakers_paginated` for bounded follow-up reads.
- `call_registry::get_calls_paginated`
  - Iterates over a bounded id range without extra loop bookkeeping.
- `outcome_manager::claim_payout`
  - Hoists fee math into shared helpers.
- `outcome_manager::batch_claim_payouts`
  - Reuses shared payout math.
  - Aggregates fee transfer into a single escrow release instead of one call
    per winner.

## Refreshing The Table

When the Soroban SDK or contract logic changes:

1. Run `cargo test stays_within_budget -- --nocapture` in
   `packages/contracts`.
2. Update the limits in the test files if the new measurements are justified.
3. Update this table to match the enforced ceilings and scenarios.

## `submit_outcome` CPU profile (SC-039)

Measured with the Soroban test budget (`measure_budget`) for one oracle
submission that reaches quorum, so it also verifies the signature, stores the
vote, finalises the outcome and calls the registry:

| Step | CPU instructions | Share |
| --- | ---: | ---: |
| `ed25519_verify` (fixed host cost) | 436,106 | 59% |
| `sha256` of the signed message | 23,978 | 3% |
| Everything else (storage, vote vector, registry call, events) | ~278,600 | 38% |
| **`submit_outcome` total** | **738,736** | 100% |

Memory is ~44 KB. `test_submit_outcome_stays_within_budget` now pins this path
at 900,000 CPU / 60,000 bytes, ~20% above the measurement, so a regression is
caught in CI.

**Result:** no change in CPU was made, and the issue's target of a >25%
reduction was **not** reached. Signature verification alone is 59% of the cost
and cannot be reduced by contract code; the 38% that remains is already a
handful of storage operations. A 25% cut would need ~185k instructions out of
that 278k, which would mean removing the vote record or the registry call, both
of which are required behaviour.

## WASM sizes (SC-040)

`[profile.release]` already sets `opt-level = "z"`, `lto = true`,
`codegen-units = 1`, `panic = "abort"`, `overflow-checks = true` and
`strip = "symbols"`, and `stellar contract build` runs the output through its
own optimiser. Running `stellar contract optimize` (wasm-opt) on every
workspace contract afterwards changed no size by more than 4 bytes, so the
requested >30% size reduction is **not achievable** with further flags: the
build is already at its floor.

| Contract | Bytes |
| --- | ---: |
| `call_registry` | 90,453 |
| `lending_pool` | 68,681 |
| `prediction_market_factory` | 59,542 |
| `parlay_betting` | 55,703 |
| `outcome_manager` | 53,923 |
| `prediction_market` | 45,817 |
| `schelling_oracle` | 25,880 |
| `backit_token` | 21,617 |
| `prediction_market_futures` | 20,049 |
| `index_fund` | 18,554 |
| `gas_station` | 17,986 |
| `charity_markets` | 17,841 |
| `tournament` | 15,291 |
| `dutch_auction` | 9,688 |
| `oracle_marketplace` | 9,015 |
| `launchpad` | 4,569 |
| `hello_world` | 626 |

Five contracts are above the issue's 50 KB figure; all are far below the 128 KiB
limit the network enforces.

What was added:

- `scripts/build-optimized.sh` (`make build-optimized`) builds with
  `stellar contract build`, optimises, and prints a size + SHA-256 table. It fails
  if any contract exceeds `MAX_WASM_BYTES` (default 131,072, the 128 KiB network
  limit; the previous script checked 256 KB and built the wrong target).
- `make wasm-size-check` runs the same check on already-built WASM, and CI runs it
  after the build so an oversized contract fails the job.
- Reproducibility: `call_registry` and `outcome_manager` were cleaned and rebuilt
  and produced byte-identical WASM (same SHA-256), as expected from a pinned
  toolchain (Rust 1.92.0) and `Cargo.lock`.
