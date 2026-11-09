#![no_main]

use chronos_engine::{Engine, EngineLimits, RecoveryLimits};
use libfuzzer_sys::fuzz_target;

const MAX_INPUT_BYTES: usize = 1_000_000;

fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > MAX_INPUT_BYTES {
        return;
    }
    let _ = Engine::recover_checkpoint(
        bytes,
        RecoveryLimits::new(MAX_INPUT_BYTES, EngineLimits::default()),
    );
});
