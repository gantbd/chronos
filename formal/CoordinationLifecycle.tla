----------------------- MODULE CoordinationLifecycle -----------------------
EXTENDS Naturals

CONSTANTS MaxIncarnation, MaxRevision, MaxTime, MaxBudget

Phases == {"absent", "ready", "running", "uncertain", "terminal"}

VARIABLES present, phase, incarnation, nextIncarnation, revision,
          now, grantBudget, grantRevoked, token, nextToken

vars == <<present, phase, incarnation, nextIncarnation, revision,
          now, grantBudget, grantRevoked, token, nextToken>>

Init ==
    /\ present = FALSE
    /\ phase = "absent"
    /\ incarnation = 0
    /\ nextIncarnation = 0
    /\ revision = 0
    /\ now = 0
    /\ grantBudget = 0
    /\ grantRevoked = TRUE
    /\ token = 0
    /\ nextToken = 1

Submit ==
    /\ ~present
    /\ nextIncarnation < MaxIncarnation
    /\ revision < MaxRevision
    /\ present' = TRUE
    /\ phase' = "ready"
    /\ incarnation' = nextIncarnation
    /\ nextIncarnation' = nextIncarnation + 1
    /\ revision' = revision + 1
    /\ UNCHANGED <<now, grantBudget, grantRevoked, token, nextToken>>

IssueGrant(budget) ==
    /\ budget \in 1..MaxBudget
    /\ grantBudget' = budget
    /\ grantRevoked' = FALSE
    /\ UNCHANGED <<present, phase, incarnation, nextIncarnation, revision,
                   now, token, nextToken>>

RevokeGrant ==
    /\ ~grantRevoked
    /\ grantRevoked' = TRUE
    /\ UNCHANGED <<present, phase, incarnation, nextIncarnation, revision,
                   now, grantBudget, token, nextToken>>

AdvanceTime(newTime) ==
    /\ newTime \in now..MaxTime
    /\ newTime > now
    /\ now' = newTime
    /\ UNCHANGED <<present, phase, incarnation, nextIncarnation, revision,
                   grantBudget, grantRevoked, token, nextToken>>

Claim ==
    /\ present
    /\ phase = "ready"
    /\ ~grantRevoked
    /\ grantBudget > 0
    /\ revision < MaxRevision
    /\ phase' = "running"
    /\ grantBudget' = grantBudget - 1
    /\ token' = nextToken
    /\ nextToken' = nextToken + 1
    /\ revision' = revision + 1
    /\ UNCHANGED <<present, incarnation, nextIncarnation, now, grantRevoked>>

FenceUnknown(lease) ==
    /\ present
    /\ phase = "running"
    /\ token = lease
    /\ revision < MaxRevision
    /\ phase' = "uncertain"
    /\ token' = 0
    /\ revision' = revision + 1
    /\ UNCHANGED <<present, incarnation, nextIncarnation, now,
                   grantBudget, grantRevoked, nextToken>>

Complete(lease) ==
    /\ present
    /\ phase = "running"
    /\ token = lease
    /\ revision < MaxRevision
    /\ phase' = "terminal"
    /\ token' = 0
    /\ revision' = revision + 1
    /\ UNCHANGED <<present, incarnation, nextIncarnation, now,
                   grantBudget, grantRevoked, nextToken>>

Resolve ==
    /\ present
    /\ phase = "uncertain"
    /\ revision < MaxRevision
    /\ phase' = "terminal"
    /\ revision' = revision + 1
    /\ UNCHANGED <<present, incarnation, nextIncarnation, now,
                   grantBudget, grantRevoked, token, nextToken>>

Cancel(ref) ==
    /\ present
    /\ ref = incarnation
    /\ phase \in {"ready", "running", "uncertain"}
    /\ revision < MaxRevision
    /\ phase' = "terminal"
    /\ token' = 0
    /\ revision' = revision + 1
    /\ UNCHANGED <<present, incarnation, nextIncarnation, now,
                   grantBudget, grantRevoked, nextToken>>

Retire(ref) ==
    /\ present
    /\ ref = incarnation
    /\ phase = "terminal"
    /\ revision < MaxRevision
    /\ present' = FALSE
    /\ phase' = "absent"
    /\ token' = 0
    /\ revision' = revision + 1
    /\ UNCHANGED <<incarnation, nextIncarnation, now,
                   grantBudget, grantRevoked, nextToken>>

Next ==
    \/ Submit
    \/ \E budget \in 1..MaxBudget : IssueGrant(budget)
    \/ RevokeGrant
    \/ \E newTime \in 0..MaxTime : AdvanceTime(newTime)
    \/ Claim
    \/ \E lease \in 1..nextToken : FenceUnknown(lease)
    \/ \E lease \in 1..nextToken : Complete(lease)
    \/ Resolve
    \/ \E ref \in 0..MaxIncarnation : Cancel(ref)
    \/ \E ref \in 0..MaxIncarnation : Retire(ref)

TypeOK ==
    /\ present \in BOOLEAN
    /\ phase \in Phases
    /\ incarnation \in 0..MaxIncarnation
    /\ nextIncarnation \in 0..MaxIncarnation
    /\ revision \in 0..MaxRevision
    /\ now \in 0..MaxTime
    /\ grantBudget \in 0..MaxBudget
    /\ grantRevoked \in BOOLEAN
    /\ token \in Nat
    /\ nextToken \in Nat

IncarnationNeverReused == present => incarnation < nextIncarnation
PresenceMatchesPhase == present <=> phase # "absent"
OnlyRunningHasLease == (token # 0) <=> phase = "running"
TimeNeverRegresses == now \in 0..MaxTime

Spec == Init /\ [][Next]_vars

=============================================================================
