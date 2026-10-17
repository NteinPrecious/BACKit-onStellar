#![no_main]

use backit_fuzz::{run_market, MarketInput};
use libfuzzer_sys::fuzz_target;

// cargo +nightly fuzz run market_settlement -- -max_total_time=60
fuzz_target!(|input: MarketInput| {
    run_market(input);
});
