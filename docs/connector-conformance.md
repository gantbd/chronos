# Connector conformance contract

Connectors implement mechanisms outside the engine and may claim only guarantees
their tests exercise. A reusable future suite must cover:

- stable identity and alias limitations;
- authentication and capability attribution;
- proof that only authenticated, current assertions are passed to grant issuance;
- grant validity selection, secure transport, refresh, global-epoch revocation,
  and capability-change revocation behavior;
- the boundary that revoking a grant blocks future claims but does not terminate
  an already-issued lease;
- request/attempt/lease correlation;
- duplicate, delayed, reordered, and lost delivery;
- cancellation versus actual quiescence;
- timeout with explicitly unknown effect outcome;
- idempotency-key scope, collision isolation, reuse across retries, and expiry;
- proof that automatic retry is used only when the external operation actually
  honors the admitted key;
- reconciliation evidence attribution, contradiction handling, and replay;
- acknowledgment and durability strength;
- append-only audit deduplication by `(scope, sequence)`, authenticated
  correlation binding, canonical V1 record parsing, durable-prefix acknowledgment,
  restart sequence restoration, and crash handling before/after append;
- optimistic checkpoint compare-and-swap conflict, exact receipt revision, and
  truthful `Volatile`/`Process`/`Durable` survival claims;
- bounded concurrency, backpressure, and cleanup;
- secret/content redaction from metrics and errors;
- restart and crash points before effect, after effect, and before acknowledgment.

Engine acceptance of a connector receipt proves structural attribution only. It
does not prove that the connector or external system reported truthfully.
Likewise, engine issuance proves bounded structural validity, not that the host
authenticated the asserted principal or capabilities.
