# Compatibility fixtures

Keep immutable examples of every released persisted and dynamic command version
here. Add valid, boundary, corrupt, truncated, unknown-version, and interrupted-
migration cases before declaring the first format supported.

`checkpoint-v1-minimal.hex` is the immutable canonical empty-engine checkpoint
for recovery format V1. Compatibility tests must continue decoding it and must
not rewrite it when later versions are introduced.

`checkpoint-v2-minimal.hex` is the immutable canonical empty-engine checkpoint
for recovery format V2, including lifecycle/revision/time high-water state and
the bounded active-grant limit. The current encoder must reproduce it exactly;
recovery tests exercise both released fixtures.
