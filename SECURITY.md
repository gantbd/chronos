# Security policy

## Supported versions

Chronos has not made a production release. The `main` branch is the only line
currently considered for security fixes; historical source candidates and
untagged archives are unsupported.

## Reporting a vulnerability

Do not disclose a suspected vulnerability in a public issue, discussion, pull
request, chat room, fixture, or evidence artifact. Use GitHub private
vulnerability reporting when available, or contact the repository owner
privately through their GitHub profile to establish a confidential channel.

Include the affected revision, impact, preconditions, and a minimal synthetic
reproduction. Do not attach credentials, private task content, checkpoints, or
unredacted journal and audit records.

## Security boundary

Chronos coordinates bounded scheduling state. It is not an identity provider,
credential verifier, agent sandbox, secret vault, remote execution service, or
durability guarantee for external effects. The standalone profile relies on a
private local OS-account boundary and explicit owner enablement. See
`docs/assurance.md`, `docs/operating-rigor.md`, and
`docs/release-readiness-v0.3.md` for current controls and residual limits.
