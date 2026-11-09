---------------------------- MODULE LeaseProtocol ----------------------------
EXTENDS Naturals, FiniteSets

CONSTANTS Tasks, Agents, MaxAttempts, Idempotent

TaskStates == {"ready", "running", "uncertain", "succeeded", "failed", "cancelled"}

VARIABLES state, owner, token, nextToken, attempts, expiry, now,
          grantEpoch, grantIssued, grantIssuedEpoch, grantExpiry
vars == <<state, owner, token, nextToken, attempts, expiry, now,
          grantEpoch, grantIssued, grantIssuedEpoch, grantExpiry>>

Init ==
    /\ state = [t \in Tasks |-> "ready"]
    /\ owner = [t \in Tasks |-> "none"]
    /\ token = [t \in Tasks |-> 0]
    /\ nextToken = 1
    /\ attempts = [t \in Tasks |-> 0]
    /\ expiry = [t \in Tasks |-> 0]
    /\ now = 0
    /\ grantEpoch = 0
    /\ grantIssued = [a \in Agents |-> FALSE]
    /\ grantIssuedEpoch = [a \in Agents |-> 0]
    /\ grantExpiry = [a \in Agents |-> 0]

IssueGrant(a, duration) ==
    /\ duration > 0
    /\ grantIssued' = [grantIssued EXCEPT ![a] = TRUE]
    /\ grantIssuedEpoch' = [grantIssuedEpoch EXCEPT ![a] = grantEpoch]
    /\ grantExpiry' = [grantExpiry EXCEPT ![a] = now + duration]
    /\ UNCHANGED <<state, owner, token, nextToken, attempts, expiry, now, grantEpoch>>

RevokeAllGrants ==
    /\ grantEpoch' = grantEpoch + 1
    /\ UNCHANGED <<state, owner, token, nextToken, attempts, expiry, now,
                   grantIssued, grantIssuedEpoch, grantExpiry>>

Claim(t, a, duration) ==
    /\ grantIssued[a]
    /\ grantIssuedEpoch[a] = grantEpoch
    /\ now < grantExpiry[a]
    /\ state[t] = "ready"
    /\ attempts[t] < MaxAttempts
    /\ duration > 0
    /\ state' = [state EXCEPT ![t] = "running"]
    /\ owner' = [owner EXCEPT ![t] = a]
    /\ token' = [token EXCEPT ![t] = nextToken]
    /\ nextToken' = nextToken + 1
    /\ attempts' = [attempts EXCEPT ![t] = @ + 1]
    /\ expiry' = [expiry EXCEPT ![t] = now + duration]
    /\ UNCHANGED <<now, grantEpoch, grantIssued, grantIssuedEpoch, grantExpiry>>

Heartbeat(t, lease, duration) ==
    /\ state[t] = "running"
    /\ token[t] = lease
    /\ now < expiry[t]
    /\ duration > 0
    /\ expiry' = [expiry EXCEPT ![t] = now + duration]
    /\ UNCHANGED <<state, owner, token, nextToken, attempts, now,
                   grantEpoch, grantIssued, grantIssuedEpoch, grantExpiry>>

Complete(t, lease) ==
    /\ state[t] = "running"
    /\ token[t] = lease
    /\ now < expiry[t]
    /\ state' = [state EXCEPT ![t] = "succeeded"]
    /\ owner' = [owner EXCEPT ![t] = "none"]
    /\ token' = [token EXCEPT ![t] = 0]
    /\ UNCHANGED <<nextToken, attempts, expiry, now,
                   grantEpoch, grantIssued, grantIssuedEpoch, grantExpiry>>

RetryFailure(t, lease) ==
    /\ state[t] = "running"
    /\ token[t] = lease
    /\ now < expiry[t]
    /\ state' = [state EXCEPT ![t] = IF attempts[t] < MaxAttempts THEN "ready" ELSE "failed"]
    /\ owner' = [owner EXCEPT ![t] = "none"]
    /\ token' = [token EXCEPT ![t] = 0]
    /\ UNCHANGED <<nextToken, attempts, expiry, now,
                   grantEpoch, grantIssued, grantIssuedEpoch, grantExpiry>>

Cancel(t) ==
    /\ state[t] \in {"ready", "running", "uncertain"}
    /\ state' = [state EXCEPT ![t] = "cancelled"]
    /\ owner' = [owner EXCEPT ![t] = "none"]
    /\ token' = [token EXCEPT ![t] = 0]
    /\ UNCHANGED <<nextToken, attempts, expiry, now,
                   grantEpoch, grantIssued, grantIssuedEpoch, grantExpiry>>

Expire(t) ==
    /\ state[t] = "running"
    /\ now >= expiry[t]
    /\ state' = [state EXCEPT ![t] =
        IF t \in Idempotent /\ attempts[t] < MaxAttempts
        THEN "ready"
        ELSE "uncertain"]
    /\ owner' = [owner EXCEPT ![t] = "none"]
    /\ token' = [token EXCEPT ![t] = 0]
    /\ UNCHANGED <<nextToken, attempts, expiry, now,
                   grantEpoch, grantIssued, grantIssuedEpoch, grantExpiry>>

ResolveCompleted(t) ==
    /\ state[t] = "uncertain"
    /\ state' = [state EXCEPT ![t] = "succeeded"]
    /\ UNCHANGED <<owner, token, nextToken, attempts, expiry, now,
                   grantEpoch, grantIssued, grantIssuedEpoch, grantExpiry>>

ResolveFailed(t, retry) ==
    /\ state[t] = "uncertain"
    /\ state' = [state EXCEPT ![t] =
        IF retry /\ attempts[t] < MaxAttempts THEN "ready" ELSE "failed"]
    /\ UNCHANGED <<owner, token, nextToken, attempts, expiry, now,
                   grantEpoch, grantIssued, grantIssuedEpoch, grantExpiry>>

Tick ==
    /\ now' = now + 1
    /\ UNCHANGED <<state, owner, token, nextToken, attempts, expiry,
                   grantEpoch, grantIssued, grantIssuedEpoch, grantExpiry>>

Next ==
    \/ \E a \in Agents, d \in 1..2 : IssueGrant(a, d)
    \/ RevokeAllGrants
    \/ \E t \in Tasks, a \in Agents, d \in 1..2 : Claim(t, a, d)
    \/ \E t \in Tasks, lease \in 1..nextToken, d \in 1..2 : Heartbeat(t, lease, d)
    \/ \E t \in Tasks, lease \in 1..nextToken : Complete(t, lease)
    \/ \E t \in Tasks, lease \in 1..nextToken : RetryFailure(t, lease)
    \/ \E t \in Tasks : Cancel(t)
    \/ \E t \in Tasks : Expire(t)
    \/ \E t \in Tasks : ResolveCompleted(t)
    \/ \E t \in Tasks, retry \in BOOLEAN : ResolveFailed(t, retry)
    \/ Tick

TypeOK ==
    /\ state \in [Tasks -> TaskStates]
    /\ owner \in [Tasks -> (Agents \cup {"none"})]
    /\ token \in [Tasks -> Nat]
    /\ nextToken \in Nat
    /\ attempts \in [Tasks -> 0..MaxAttempts]
    /\ expiry \in [Tasks -> Nat]
    /\ now \in Nat
    /\ grantEpoch \in Nat
    /\ grantIssued \in [Agents -> BOOLEAN]
    /\ grantIssuedEpoch \in [Agents -> Nat]
    /\ grantExpiry \in [Agents -> Nat]

TerminalHasNoOwner ==
    \A t \in Tasks : state[t] \in {"uncertain", "succeeded", "failed", "cancelled"} =>
        /\ owner[t] = "none"
        /\ token[t] = 0

Spec == Init /\ [][Next]_vars

=============================================================================
