//! Optional bounded ephemeral change history. Observation failure never changes
//! command outcomes. Durable interpreters publish only after a strong receipt.
use crate::{Authorizer, Service, increment};
use chronos_engine::{Engine, StateRevision, TaskListEntryV1};
use chronos_host::native_observer::*;
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug)]
pub struct FeedLimits {
    pub max_cached_tasks: usize,
    pub max_cached_bytes: usize,
    pub max_transitions: usize,
    pub max_history_bytes: usize,
    pub max_changes_per_transition: usize,
    pub max_page_transitions: usize,
    pub max_response_bytes: usize,
}
/// Byte budgets charge canonical content plus conservative fixed metadata;
/// these are not allocator/RSS or physical-device guarantees.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeedConfigError {
    InvalidLimits,
}
pub trait ObserverAuthorizer: Authorizer {
    /// Current authority for the entire query scope, including historical
    /// membership. Deny when that complete scope cannot be established.
    fn allows_observer(&mut self, request: &ObserverRequestV1) -> bool;
    /// Separate release check over the actual historical resources. Old task
    /// membership or current execute rights alone never imply disclosure.
    fn allows_observer_changes(&mut self, changes: &[ChangeSetV1]) -> bool;
}
#[derive(Clone, Copy, Debug, Default)]
pub struct FeedSignals {
    pub queries: u64,
    pub unauthorized: u64,
    pub gaps: u64,
    pub limited: u64,
    pub published: u64,
    pub evicted: u64,
    pub overflows: u64,
    pub service_time: std::time::Duration,
    pub saturated: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FeedUsage {
    /// None means the configured cache budget could not capture the state.
    pub cached_tasks: Option<usize>,
    pub history_transitions: usize,
    pub history_bytes: usize,
    pub max_history_transitions: usize,
    pub max_history_bytes: usize,
}
pub(crate) struct Feed {
    generation: ObserverGeneration,
    head: StateRevision,
    cached: Option<Vec<TaskListEntryV1>>,
    history: VecDeque<(ChangeSetV1, usize)>,
    history_bytes: usize,
    limits: FeedLimits,
    signals: FeedSignals,
}
impl Feed {
    fn new(engine: &Engine, limits: FeedLimits) -> Result<Self, FeedConfigError> {
        if limits.max_cached_tasks == 0
            || limits.max_cached_bytes == 0
            || limits.max_transitions == 0
            || limits.max_history_bytes == 0
            || limits.max_changes_per_transition == 0
            || limits.max_page_transitions == 0
            || limits.max_response_bytes <= 32
        {
            return Err(FeedConfigError::InvalidLimits);
        }
        Ok(Self {
            generation: ObserverGeneration::new(),
            head: engine.task_state_revision(),
            cached: capture(engine, limits),
            history: VecDeque::new(),
            history_bytes: 0,
            limits,
            signals: FeedSignals::default(),
        })
    }
    fn cursor(&self, revision: StateRevision) -> ObserverCursorV1 {
        ObserverCursorV1::from_observation(&self.generation, revision)
    }
    fn reset_history(&mut self) {
        self.history.clear();
        self.history_bytes = 0;
        increment(&mut self.signals.overflows, &mut self.signals.saturated);
    }
    pub(crate) fn publish(&mut self, engine: &Engine) {
        let to = engine.task_state_revision();
        if to == self.head {
            return;
        }
        let from = self.head;
        self.head = to;
        let after = capture(engine, self.limits);
        let changes = self
            .cached
            .as_ref()
            .zip(after.as_ref())
            .and_then(|(before, after)| diff(before, after, self.limits));
        self.cached = after;
        let Some((changes, cost)) = changes else {
            self.reset_history();
            return;
        };
        if cost > self.limits.max_history_bytes
            || cost > self.limits.max_response_bytes.saturating_sub(32)
            || self.history.try_reserve(1).is_err()
        {
            self.reset_history();
            return;
        }
        while self.history.len() >= self.limits.max_transitions
            || cost
                > self
                    .limits
                    .max_history_bytes
                    .saturating_sub(self.history_bytes)
        {
            if let Some((_, cost)) = self.history.pop_front() {
                self.history_bytes -= cost;
                increment(&mut self.signals.evicted, &mut self.signals.saturated);
            } else {
                self.reset_history();
                return;
            }
        }
        self.history
            .push_back((ChangeSetV1 { from, to, changes }, cost));
        self.history_bytes += cost;
        increment(&mut self.signals.published, &mut self.signals.saturated);
    }
    fn read(&mut self, request: &ObserverRequestV1) -> ObserverReplyV1 {
        if request.limit == 0 || request.limit > self.limits.max_page_transitions {
            increment(&mut self.signals.limited, &mut self.signals.saturated);
            return Err(ObserverRefusalV1::Limit);
        }
        if self.head == StateRevision::Exhausted {
            return Err(ObserverRefusalV1::Exhausted);
        }
        let gap = |feed: &mut Self| {
            increment(&mut feed.signals.gaps, &mut feed.signals.saturated);
            Ok(ObserverResponseV1::ResyncRequired {
                head: feed.cursor(feed.head),
            })
        };
        let Some(after) = &request.after else {
            return gap(self);
        };
        if !after.belongs_to(&self.generation) {
            return gap(self);
        }
        if after.revision() == self.head {
            return Ok(ObserverResponseV1::Changes {
                cursor: self.cursor(self.head),
                changes: Vec::new(),
                more: false,
            });
        }
        let Some(start) = self
            .history
            .iter()
            .position(|(change, _)| change.from == after.revision())
        else {
            return gap(self);
        };
        let mut changes = Vec::new();
        changes
            .try_reserve(request.limit.min(self.history.len()))
            .map_err(|_| ObserverRefusalV1::Capacity)?;
        let mut used = 32_usize;
        let mut revision = after.revision();
        for (entry, cost) in self.history.iter().skip(start).take(request.limit) {
            if entry.from != revision {
                return gap(self);
            }
            if *cost > self.limits.max_response_bytes.saturating_sub(used) {
                break;
            }
            used += cost;
            revision = entry.to;
            changes.push(entry.clone());
        }
        if changes.is_empty() {
            return Err(ObserverRefusalV1::Limit);
        }
        Ok(ObserverResponseV1::Changes {
            cursor: self.cursor(revision),
            changes,
            more: revision != self.head,
        })
    }
}
fn capture(engine: &Engine, limits: FeedLimits) -> Option<Vec<TaskListEntryV1>> {
    let mut entries = Vec::new();
    let mut bytes = 0_usize;
    for task in engine.tasks() {
        if entries.len() >= limits.max_cached_tasks {
            return None;
        }
        bytes = bytes.checked_add(task.id.as_str().len().checked_add(64)?)?;
        if bytes > limits.max_cached_bytes {
            return None;
        }
        entries.try_reserve(1).ok()?;
        entries.push(TaskListEntryV1 {
            reference: engine.task_reference(&task.id)?,
            task: engine.task_snapshot(&task.id)?,
        });
    }
    Some(entries)
}
fn diff(
    before: &[TaskListEntryV1],
    after: &[TaskListEntryV1],
    limits: FeedLimits,
) -> Option<(Vec<TaskChangeV1>, usize)> {
    let mut changes = Vec::new();
    let mut cost = 32_usize;
    let (mut old, mut new) = (0, 0);
    let mut push = |change: TaskChangeV1| -> Option<()> {
        if changes.len() >= limits.max_changes_per_transition {
            return None;
        }
        let size = match &change {
            TaskChangeV1::Upsert(entry) => {
                entry.reference.task_id().as_str().len().checked_add(64)?
            }
            TaskChangeV1::Removed(reference) => {
                reference.task_id().as_str().len().checked_add(32)?
            }
        };
        cost = cost.checked_add(size)?;
        if cost > limits.max_history_bytes || cost > limits.max_response_bytes.saturating_sub(32) {
            return None;
        }
        changes.try_reserve(1).ok()?;
        changes.push(change);
        Some(())
    };
    while old < before.len() || new < after.len() {
        match (before.get(old), after.get(new)) {
            (Some(previous), Some(current)) => match previous
                .reference
                .task_id()
                .cmp(current.reference.task_id())
            {
                std::cmp::Ordering::Less => {
                    push(TaskChangeV1::Removed(previous.reference.clone()))?;
                    old += 1;
                }
                std::cmp::Ordering::Greater => {
                    push(TaskChangeV1::Upsert(current.clone()))?;
                    new += 1;
                }
                std::cmp::Ordering::Equal => {
                    if previous.reference != current.reference {
                        push(TaskChangeV1::Removed(previous.reference.clone()))?;
                    }
                    if previous != current {
                        push(TaskChangeV1::Upsert(current.clone()))?;
                    }
                    old += 1;
                    new += 1;
                }
            },
            (Some(previous), None) => {
                push(TaskChangeV1::Removed(previous.reference.clone()))?;
                old += 1;
            }
            (None, Some(current)) => {
                push(TaskChangeV1::Upsert(current.clone()))?;
                new += 1;
            }
            (None, None) => break,
        }
    }
    Some((changes, cost))
}
impl<A: Authorizer> Service<A> {
    pub fn enable_observer(&mut self, limits: FeedLimits) -> Result<(), FeedConfigError> {
        self.observer = Some(Feed::new(&self.engine, limits)?);
        Ok(())
    }
    pub fn disable_observer(&mut self) {
        self.observer = None;
    }
    pub fn observer_signals(&self) -> Option<FeedSignals> {
        self.observer.as_ref().map(|feed| feed.signals)
    }
    pub fn observer_usage(&self) -> Option<FeedUsage> {
        self.observer.as_ref().map(|feed| FeedUsage {
            cached_tasks: feed.cached.as_ref().map(Vec::len),
            history_transitions: feed.history.len(),
            history_bytes: feed.history_bytes,
            max_history_transitions: feed.limits.max_transitions,
            max_history_bytes: feed.limits.max_history_bytes,
        })
    }
    pub(crate) fn publish_observer(&mut self) {
        if let Some(feed) = &mut self.observer {
            feed.publish(&self.engine);
        }
    }
    pub(crate) fn new_gated(engine: Engine, authorizer: A, limits: crate::Limits) -> Self {
        let mut service = Self::new(engine, authorizer, limits);
        service.gated_observer = true;
        service
    }
}
impl<A: ObserverAuthorizer> Service<A> {
    pub fn observe(&mut self, request: ObserverRequestV1) -> ObserverReplyV1 {
        let started = std::time::Instant::now();
        if let Some(feed) = &mut self.observer {
            increment(&mut feed.signals.queries, &mut feed.signals.saturated);
        }
        let response = if !self.authorizer.allows_observer(&request) {
            Err(ObserverRefusalV1::Unauthorized)
        } else {
            match &mut self.observer {
                Some(feed) => feed.read(&request),
                None => Err(ObserverRefusalV1::Unavailable),
            }
        };
        let response = match response {
            Ok(ObserverResponseV1::Changes {
                cursor,
                changes,
                more,
            }) => {
                if self.authorizer.allows_observer(&request)
                    && self.authorizer.allows_observer_changes(&changes)
                {
                    Ok(ObserverResponseV1::Changes {
                        cursor,
                        changes,
                        more,
                    })
                } else {
                    Err(ObserverRefusalV1::Unauthorized)
                }
            }
            Ok(response) => {
                if self.authorizer.allows_observer(&request) {
                    Ok(response)
                } else {
                    Err(ObserverRefusalV1::Unauthorized)
                }
            }
            other => other,
        };
        if let Some(feed) = &mut self.observer {
            if matches!(&response, Err(ObserverRefusalV1::Unauthorized)) {
                increment(&mut feed.signals.unauthorized, &mut feed.signals.saturated);
            }
            match feed.signals.service_time.checked_add(started.elapsed()) {
                Some(total) => feed.signals.service_time = total,
                None => {
                    feed.signals.service_time = std::time::Duration::MAX;
                    feed.signals.saturated = true;
                }
            }
        }
        response
    }
}
