//! Headless, borrowed views over an already bounded/validated observer snapshot.
//! Filtering is not authorization, claim eligibility, UI layout or a new owner.
use crate::observer::Observer;
use chronos_protocol::{Entry, Phase, Revision};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionKindV1 {
    All,
    Ready,
    Blocked,
    Running,
    Uncertain,
    Terminal,
}
impl ProjectionKindV1 {
    fn matches(self, phase: &Phase) -> bool {
        match self {
            Self::All => true,
            Self::Ready => matches!(phase, Phase::Ready),
            Self::Blocked => matches!(phase, Phase::Blocked),
            Self::Running => matches!(phase, Phase::Running),
            Self::Uncertain => matches!(phase, Phase::Uncertain),
            Self::Terminal => matches!(phase, Phase::Succeeded | Phase::Failed | Phase::Cancelled),
        }
    }
    fn index(self) -> usize {
        match self {
            Self::All => 0,
            Self::Ready => 1,
            Self::Blocked => 2,
            Self::Running => 3,
            Self::Uncertain => 4,
            Self::Terminal => 5,
        }
    }
}
pub struct ProjectionV1<'a> {
    revision: &'a Revision,
    entries: &'a [Entry],
    kind: ProjectionKindV1,
    matched: usize,
}
impl<'a> ProjectionV1<'a> {
    /// Observation revision, never a continuing currentness/permission promise.
    pub fn revision(&self) -> &Revision {
        self.revision
    }
    /// No new task allocation/copy or I/O. At most the observer's entry ceiling
    /// is scanned per traversal; canonical lexical ordering is preserved.
    pub fn rows(&self) -> impl Iterator<Item = &'a Entry> + '_ {
        self.entries
            .iter()
            .filter(|entry| self.kind.matches(&entry.snapshot.phase))
    }
    pub fn len(&self) -> usize {
        self.matched
    }
    pub fn is_empty(&self) -> bool {
        self.matched == 0
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct ProjectionSignalsV1 {
    pub calls: u64,
    /// All/Ready/Blocked/Running/Uncertain/Terminal; no domain labels.
    pub by_kind: [u64; 6],
    pub unavailable: u64,
    pub scanned: u64,
    pub matched: u64,
    /// Projection CPU/residence only, not backend/queue/executor latency.
    pub residence: std::time::Duration,
    pub saturated: bool,
}
fn add(counter: &mut u64, value: usize, saturated: &mut bool) {
    if let Some(total) = u64::try_from(value)
        .ok()
        .and_then(|v| counter.checked_add(v))
    {
        *counter = total;
    } else {
        *counter = u64::MAX;
        *saturated = true;
    }
}
/// Uses only data the observer has already disclosed; requests no resource and
/// caches no permissions. A failed refresh/invalidation yields None for every
/// kind. Borrowing prevents refresh/invalidation while this projection is live;
/// callers still own any copies previously made of disclosed data.
pub fn project<'a>(
    observer: &'a Observer,
    kind: ProjectionKindV1,
    signals: &mut ProjectionSignalsV1,
) -> Option<ProjectionV1<'a>> {
    let started = std::time::Instant::now();
    add(&mut signals.calls, 1, &mut signals.saturated);
    add(
        &mut signals.by_kind[kind.index()],
        1,
        &mut signals.saturated,
    );
    let projection = observer.view().map(|view| {
        let matched = view
            .entries
            .iter()
            .filter(|entry| kind.matches(&entry.snapshot.phase))
            .count();
        add(
            &mut signals.scanned,
            view.entries.len(),
            &mut signals.saturated,
        );
        add(&mut signals.matched, matched, &mut signals.saturated);
        ProjectionV1 {
            revision: &view.revision,
            entries: &view.entries,
            kind,
            matched,
        }
    });
    if projection.is_none() {
        add(&mut signals.unavailable, 1, &mut signals.saturated);
    }
    if let Some(total) = signals.residence.checked_add(started.elapsed()) {
        signals.residence = total;
    } else {
        signals.residence = std::time::Duration::MAX;
        signals.saturated = true;
    }
    projection
}
