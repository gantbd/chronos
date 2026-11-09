# Coverage-guided fuzzing

`checkpoint_decode` presents at most 1,000,000 bytes to the same bounded V1
recovery entry point used by hosts. The oracle is total typed return without
panic, abort, or unbounded decoder-controlled allocation. Semantic valid-state
correctness remains covered by round-trip, golden, and reference scenarios.

Run with a pinned `cargo-fuzz`/libFuzzer toolchain, for example:

```sh
cargo fuzz run checkpoint_decode -- -runs=100000
```

Every minimized crash or semantic regression becomes a normal repository
fixture before the fix is accepted. Fuzzer dependencies are isolated from the
engine manifest and are not part of the runtime dependency graph.
