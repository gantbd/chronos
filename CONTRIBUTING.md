# Contributing to Chronos

Chronos is source-distributed and has not made a supported production release.
Read `docs/release-readiness-v0.3.md`, `docs/operating-rigor.md`, and
`SECURITY.md` before changing behavior or claims.

Run the complete non-GUI workspace gate before submitting changes:

```sh
make check
```

The sibling TUI, GUI, and excluded Vellum-capture integration have separate
qualification. Their absence or failure is never credited to the core gate.

- Preserve closed, bounded, versioned domains and content-safe diagnostics.
- Keep optional evidence and historical readiness ledgers scoped to their exact
  source profiles.
- Do not enable registry publication or add a distribution claim without an
  explicit policy change.
- Do not commit credentials, private data, local paths, or build output.

Report suspected vulnerabilities through `SECURITY.md`, not a public issue.
