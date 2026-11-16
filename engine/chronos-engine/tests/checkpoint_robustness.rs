use chronos_engine::{Engine, EngineLimits, RecoveryLimits};

fn fixture(hex: &str) -> Vec<u8> {
    hex.trim()
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

fn assert_total_decoder(bytes: &[u8]) {
    if let Ok(engine) =
        Engine::recover_checkpoint(bytes, RecoveryLimits::new(1_024, EngineLimits::default()))
    {
        assert!(engine.resource_usage().fits_within(engine.resource_limit()));
        assert!(engine.tasks().count() <= engine.limits().max_tasks);
        assert!(engine.retained_outcomes().count() <= engine.limits().max_retained_outcomes);
    }
}

#[test]
fn every_released_fixture_truncation_is_a_typed_refusal() {
    for fixture in [
        fixture(include_str!("fixtures/checkpoint-v1-minimal.hex")),
        fixture(include_str!("fixtures/checkpoint-v2-minimal.hex")),
    ] {
        for length in 0..fixture.len() {
            assert!(
                Engine::recover_checkpoint(
                    &fixture[..length],
                    RecoveryLimits::new(1_024, EngineLimits::default()),
                )
                .is_err(),
                "truncation at {length} unexpectedly recovered"
            );
        }
        assert_total_decoder(&fixture);
    }
}

#[test]
fn systematic_single_byte_mutations_keep_the_decoder_total() {
    for fixture in [
        fixture(include_str!("fixtures/checkpoint-v1-minimal.hex")),
        fixture(include_str!("fixtures/checkpoint-v2-minimal.hex")),
    ] {
        for index in 0..fixture.len() {
            for replacement in [0_u8, 0xff, fixture[index] ^ 0x5a] {
                let mut mutated = fixture.clone();
                mutated[index] = replacement;
                assert_total_decoder(&mutated);
            }
        }
    }
}

#[test]
fn deterministic_generated_bytes_keep_the_decoder_total_and_bounded() {
    let mut seed = 0x5eed_cafe_f00d_u64;
    for _ in 0..2_048 {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let length = usize::try_from(seed % 513).unwrap();
        let mut bytes = Vec::with_capacity(length);
        for _ in 0..length {
            seed = seed.wrapping_mul(1_442_695_040_888_963_407).wrapping_add(1);
            bytes.push(seed.to_le_bytes()[3]);
        }
        assert_total_decoder(&bytes);
    }
}
