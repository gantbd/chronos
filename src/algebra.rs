//! Closed algebraic domains used by engine contracts.

use crate::model::StateRevision;

/// A finite domain with total least-upper-bound and greatest-lower-bound
/// operations plus explicit bottom and top elements.
pub trait BoundedLattice: Copy + Eq {
    fn bottom() -> Self;
    fn top() -> Self;
    fn join(self, other: Self) -> Self;
    fn meet(self, other: Self) -> Self;

    /// The partial order induced by `join`.
    fn leq(self, other: Self) -> bool {
        self.join(other) == other
    }
}

impl BoundedLattice for StateRevision {
    fn bottom() -> Self {
        Self::ZERO
    }

    fn top() -> Self {
        Self::Exhausted
    }

    fn join(self, other: Self) -> Self {
        self.max(other)
    }

    fn meet(self, other: Self) -> Self {
        self.min(other)
    }
}

/// A closed associative composition with an identity element.
pub trait Monoid: Sized {
    fn identity() -> Self;
    fn combine(&self, other: &Self) -> Self;
}

/// Optional greatest element under a total order.
///
/// `Empty` is identity and candidate composition selects `max`. This is a
/// commutative idempotent monoid when `Ord` is lawful.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Best<T> {
    Empty,
    Candidate(T),
}

impl<T> Best<T> {
    pub fn candidate(value: T) -> Self {
        Self::Candidate(value)
    }

    pub fn into_option(self) -> Option<T> {
        match self {
            Self::Empty => None,
            Self::Candidate(value) => Some(value),
        }
    }
}

impl<T: Clone + Ord> Monoid for Best<T> {
    fn identity() -> Self {
        Self::Empty
    }

    fn combine(&self, other: &Self) -> Self {
        match (self, other) {
            (Self::Empty, value) | (value, Self::Empty) => value.clone(),
            (Self::Candidate(left), Self::Candidate(right)) => {
                Self::Candidate(left.max(right).clone())
            }
        }
    }
}

/// A capacity-bounded completion of the free monoid over `T`.
///
/// `Word` preserves generator order. Any construction or composition beyond
/// `N` enters the explicit absorbing `Overflow` value, keeping composition total
/// and closed without truncating or silently dropping generators. Representation
/// allocation failure is the distinct, stronger absorbing `Capacity` value.
/// Neither refusal value is executable as a plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BoundedWord<T, const N: usize> {
    Word(Vec<T>),
    Overflow,
    Capacity,
}

impl<T, const N: usize> BoundedWord<T, N> {
    pub fn singleton(item: T) -> Self {
        if N == 0 {
            Self::Overflow
        } else {
            Self::Word(vec![item])
        }
    }

    pub fn from_items(items: impl IntoIterator<Item = T>) -> Self {
        let mut word = Vec::new();
        for item in items {
            if word.len() == N {
                return Self::Overflow;
            }
            if word.try_reserve(1).is_err() {
                return Self::Capacity;
            }
            word.push(item);
        }
        Self::Word(word)
    }

    pub fn as_slice(&self) -> Option<&[T]> {
        match self {
            Self::Word(items) => Some(items),
            Self::Overflow | Self::Capacity => None,
        }
    }

    pub fn is_overflow(&self) -> bool {
        matches!(self, Self::Overflow)
    }

    pub fn is_capacity_fault(&self) -> bool {
        matches!(self, Self::Capacity)
    }

    /// Consuming composition that moves generators and therefore does not
    /// require a potentially allocating `T: Clone` implementation.
    pub fn concat(self, other: Self) -> Self {
        if matches!(self, Self::Capacity) || matches!(other, Self::Capacity) {
            return Self::Capacity;
        }
        let (Self::Word(mut left), Self::Word(right)) = (self, other) else {
            return Self::Overflow;
        };
        let Some(length) = left.len().checked_add(right.len()) else {
            return Self::Overflow;
        };
        if length > N {
            return Self::Overflow;
        }
        if left.try_reserve_exact(right.len()).is_err() {
            return Self::Capacity;
        }
        left.extend(right);
        Self::Word(left)
    }
}

impl<T: Clone, const N: usize> Monoid for BoundedWord<T, N> {
    fn identity() -> Self {
        Self::Word(Vec::new())
    }

    fn combine(&self, other: &Self) -> Self {
        if matches!(self, Self::Capacity) || matches!(other, Self::Capacity) {
            return Self::Capacity;
        }
        let (Self::Word(left), Self::Word(right)) = (self, other) else {
            return Self::Overflow;
        };
        let Some(length) = left.len().checked_add(right.len()) else {
            return Self::Overflow;
        };
        if length > N {
            return Self::Overflow;
        }
        let mut combined = Vec::new();
        if combined.try_reserve_exact(length).is_err() {
            return Self::Capacity;
        }
        combined.extend_from_slice(left);
        combined.extend_from_slice(right);
        Self::Word(combined)
    }
}

/// Knowledge about one attributed external effect.
///
/// This is the four-element diamond lattice. `Unknown` is no evidence,
/// `Failed` and `Completed` are incomparable observations, and `Conflict` is
/// contradictory evidence. `Conflict` is top in the knowledge order; it is not
/// successful completion and grants no scheduling authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EffectKnowledge {
    Unknown,
    Failed,
    Completed,
    Conflict,
}

impl BoundedLattice for EffectKnowledge {
    fn bottom() -> Self {
        Self::Unknown
    }

    fn top() -> Self {
        Self::Conflict
    }

    fn join(self, other: Self) -> Self {
        use EffectKnowledge::{Completed, Conflict, Failed, Unknown};
        match (self, other) {
            (Failed, Failed) => Failed,
            (Completed, Completed) => Completed,
            (Unknown, value) | (value, Unknown) => value,
            (Conflict, _) | (_, Conflict) | (Completed, Failed) | (Failed, Completed) => Conflict,
        }
    }

    fn meet(self, other: Self) -> Self {
        use EffectKnowledge::{Completed, Conflict, Failed, Unknown};
        match (self, other) {
            (Failed, Failed) => Failed,
            (Completed, Completed) => Completed,
            (Conflict, value) | (value, Conflict) => value,
            (Unknown, _) | (_, Unknown) | (Completed, Failed) | (Failed, Completed) => Unknown,
        }
    }
}

/// Allocation-free powerset of a closed universe containing `N` indexed atoms.
///
/// Union is join, intersection is meet, the empty set is bottom, and the full
/// universe is top. It is both a bounded distributive lattice and a Boolean
/// algebra under `complement`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FinitePowerset<const N: usize> {
    members: [bool; N],
}

impl<const N: usize> FinitePowerset<N> {
    pub fn from_membership(members: [bool; N]) -> Self {
        Self { members }
    }

    pub fn singleton(index: usize) -> Option<Self> {
        if index >= N {
            return None;
        }
        let mut members = [false; N];
        members[index] = true;
        Some(Self { members })
    }

    pub fn contains(&self, index: usize) -> bool {
        self.members.get(index).copied().unwrap_or(false)
    }

    pub fn complement(self) -> Self {
        Self {
            members: std::array::from_fn(|index| !self.members[index]),
        }
    }
}

impl<const N: usize> BoundedLattice for FinitePowerset<N> {
    fn bottom() -> Self {
        Self {
            members: [false; N],
        }
    }

    fn top() -> Self {
        Self { members: [true; N] }
    }

    fn join(self, other: Self) -> Self {
        Self {
            members: std::array::from_fn(|index| self.members[index] || other.members[index]),
        }
    }

    fn meet(self, other: Self) -> Self {
        Self {
            members: std::array::from_fn(|index| self.members[index] && other.members[index]),
        }
    }
}

/// Component-wise logical resource measure.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ResourceVector {
    pub tasks: usize,
    pub dependency_edges: usize,
    pub capability_entries: usize,
    pub payload_bytes: usize,
    pub retained_outcomes: usize,
    pub active_grants: usize,
}

/// Closed resource-accounting carrier.
///
/// `Overflow` is greater than every finite vector and absorbs composition. It is
/// a refusal/fault value, not an infinite usable budget.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceAccount {
    Finite(ResourceVector),
    Overflow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceResidualError {
    Underflow,
    Indeterminate,
}

impl ResourceAccount {
    pub fn retained_outcomes(self) -> Option<usize> {
        match self {
            Self::Finite(vector) => Some(vector.retained_outcomes),
            Self::Overflow => None,
        }
    }

    pub fn residual(self, used: Self) -> Result<Self, ResourceResidualError> {
        let (Self::Finite(available), Self::Finite(used)) = (self, used) else {
            return Err(ResourceResidualError::Indeterminate);
        };
        Ok(Self::Finite(ResourceVector {
            tasks: available
                .tasks
                .checked_sub(used.tasks)
                .ok_or(ResourceResidualError::Underflow)?,
            dependency_edges: available
                .dependency_edges
                .checked_sub(used.dependency_edges)
                .ok_or(ResourceResidualError::Underflow)?,
            capability_entries: available
                .capability_entries
                .checked_sub(used.capability_entries)
                .ok_or(ResourceResidualError::Underflow)?,
            payload_bytes: available
                .payload_bytes
                .checked_sub(used.payload_bytes)
                .ok_or(ResourceResidualError::Underflow)?,
            retained_outcomes: available
                .retained_outcomes
                .checked_sub(used.retained_outcomes)
                .ok_or(ResourceResidualError::Underflow)?,
            active_grants: available
                .active_grants
                .checked_sub(used.active_grants)
                .ok_or(ResourceResidualError::Underflow)?,
        }))
    }

    pub fn fits_within(self, limit: Self) -> bool {
        self.leq(limit)
    }
}

impl Monoid for ResourceAccount {
    fn identity() -> Self {
        Self::Finite(ResourceVector::default())
    }

    fn combine(&self, other: &Self) -> Self {
        let (Self::Finite(left), Self::Finite(right)) = (*self, *other) else {
            return Self::Overflow;
        };
        let Some(tasks) = left.tasks.checked_add(right.tasks) else {
            return Self::Overflow;
        };
        let Some(dependency_edges) = left.dependency_edges.checked_add(right.dependency_edges)
        else {
            return Self::Overflow;
        };
        let Some(capability_entries) = left
            .capability_entries
            .checked_add(right.capability_entries)
        else {
            return Self::Overflow;
        };
        let Some(payload_bytes) = left.payload_bytes.checked_add(right.payload_bytes) else {
            return Self::Overflow;
        };
        let Some(retained_outcomes) = left.retained_outcomes.checked_add(right.retained_outcomes)
        else {
            return Self::Overflow;
        };
        let Some(active_grants) = left.active_grants.checked_add(right.active_grants) else {
            return Self::Overflow;
        };
        Self::Finite(ResourceVector {
            tasks,
            dependency_edges,
            capability_entries,
            payload_bytes,
            retained_outcomes,
            active_grants,
        })
    }
}

impl BoundedLattice for ResourceAccount {
    fn bottom() -> Self {
        Self::Finite(ResourceVector::default())
    }

    fn top() -> Self {
        Self::Overflow
    }

    fn join(self, other: Self) -> Self {
        let (Self::Finite(left), Self::Finite(right)) = (self, other) else {
            return Self::Overflow;
        };
        Self::Finite(ResourceVector {
            tasks: left.tasks.max(right.tasks),
            dependency_edges: left.dependency_edges.max(right.dependency_edges),
            capability_entries: left.capability_entries.max(right.capability_entries),
            payload_bytes: left.payload_bytes.max(right.payload_bytes),
            retained_outcomes: left.retained_outcomes.max(right.retained_outcomes),
            active_grants: left.active_grants.max(right.active_grants),
        })
    }

    fn meet(self, other: Self) -> Self {
        match (self, other) {
            (Self::Overflow, value) | (value, Self::Overflow) => value,
            (Self::Finite(left), Self::Finite(right)) => Self::Finite(ResourceVector {
                tasks: left.tasks.min(right.tasks),
                dependency_edges: left.dependency_edges.min(right.dependency_edges),
                capability_entries: left.capability_entries.min(right.capability_entries),
                payload_bytes: left.payload_bytes.min(right.payload_bytes),
                retained_outcomes: left.retained_outcomes.min(right.retained_outcomes),
                active_grants: left.active_grants.min(right.active_grants),
            }),
        }
    }
}
