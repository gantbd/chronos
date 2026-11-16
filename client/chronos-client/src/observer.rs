//! Bounded full resynchronization over existing V1 reads. No subscriptions,
//! timers, mutation retries, retained change history or permission cache.
use crate::WireClientV1;
use chronos_protocol::{Command, Cursor, Entry, Failure, Outcome, Request, Revision};
mod feed;
pub use feed::FeedSignals;

#[derive(Clone, Copy, Debug)]
pub struct ObserverLimits {
    pub page_size: usize,
    /// At most this many page requests plus one final revision check.
    pub max_pages: usize,
    pub max_entries: usize,
    pub max_id_bytes: usize,
}
pub struct View {
    pub revision: Revision,
    pub entries: Vec<Entry>,
}
pub enum RefreshError<E> {
    InvalidLimits,
    Transport(E),
    Refused,
    ResyncRequired,
    Limit,
    InvalidResponse,
    Capacity,
}
impl<E> std::fmt::Debug for RefreshError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidLimits => "InvalidLimits",
            Self::Transport(_) => "Transport",
            Self::Refused => "Refused",
            Self::ResyncRequired => "ResyncRequired",
            Self::Limit => "Limit",
            Self::InvalidResponse => "InvalidResponse",
            Self::Capacity => "Capacity",
        })
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Signals {
    pub refreshes: u64,
    pub pages: u64,
    pub published: u64,
    pub transport_failed: u64,
    pub refused: u64,
    pub resync_required: u64,
    pub limited: u64,
    pub invalid: u64,
    pub capacity_failed: u64,
    pub service_time: std::time::Duration,
    pub saturated: bool,
}
fn increment(value: &mut u64, saturated: &mut bool) {
    if let Some(next) = value.checked_add(1) {
        *value = next;
    } else {
        *saturated = true;
    }
}
pub struct Observer {
    limits: ObserverLimits,
    view: Option<View>,
    signals: Signals,
    feed_cursor: Option<chronos_protocol::coordination_v3::FeedCursor>,
    feed_signals: FeedSignals,
}
impl Observer {
    pub fn new(limits: ObserverLimits) -> Result<Self, RefreshError<std::convert::Infallible>> {
        if limits.page_size == 0
            || limits.max_pages == 0
            || limits.max_entries == 0
            || limits.max_id_bytes == 0
            || limits.page_size > limits.max_entries
        {
            return Err(RefreshError::InvalidLimits);
        }
        Ok(Self {
            limits,
            view: None,
            signals: Signals::default(),
            feed_cursor: None,
            feed_signals: FeedSignals::default(),
        })
    }
    /// Last successfully refreshed view, not a claim of continuing permission
    /// or currentness. Callers own any copies they make of disclosed data.
    pub fn view(&self) -> Option<&View> {
        self.view.as_ref()
    }
    pub fn signals(&self) -> Signals {
        self.signals
    }
    pub fn invalidate(&mut self) {
        self.view = None;
        self.feed_cursor = None;
    }
    /// One bounded attempt, never automatic retry. Any failure invalidates the
    /// old local view and discards partial data. Every page and final revision
    /// call goes through current host authorization. A concurrent mutation
    /// requires explicit caller-driven resynchronization from the beginning.
    pub fn refresh<C: WireClientV1>(
        &mut self,
        client: &mut C,
    ) -> Result<(), RefreshError<C::Error>> {
        self.invalidate();
        let started = std::time::Instant::now();
        increment(&mut self.signals.refreshes, &mut self.signals.saturated);
        let result = self.collect(client);
        let result = match result {
            Ok(view) => {
                self.view = Some(view);
                increment(&mut self.signals.published, &mut self.signals.saturated);
                Ok(())
            }
            Err(error) => {
                let counter = match &error {
                    RefreshError::Transport(_) => &mut self.signals.transport_failed,
                    RefreshError::Refused => &mut self.signals.refused,
                    RefreshError::ResyncRequired => &mut self.signals.resync_required,
                    RefreshError::Limit => &mut self.signals.limited,
                    RefreshError::Capacity => &mut self.signals.capacity_failed,
                    RefreshError::InvalidResponse | RefreshError::InvalidLimits => {
                        &mut self.signals.invalid
                    }
                };
                increment(counter, &mut self.signals.saturated);
                Err(error)
            }
        };
        match self.signals.service_time.checked_add(started.elapsed()) {
            Some(total) => self.signals.service_time = total,
            None => {
                self.signals.service_time = std::time::Duration::MAX;
                self.signals.saturated = true;
            }
        }
        result
    }
    fn collect<C: WireClientV1>(&mut self, client: &mut C) -> Result<View, RefreshError<C::Error>> {
        let mut entries: Vec<Entry> = Vec::new();
        let mut cursor: Option<Cursor> = None;
        let mut observed: Option<Revision> = None;
        for _ in 0..self.limits.max_pages {
            increment(&mut self.signals.pages, &mut self.signals.saturated);
            let response = client
                .request(Request {
                    version: 1,
                    command: Command::Page {
                        cursor: cursor.clone(),
                        limit: self.limits.page_size,
                    },
                })
                .map_err(RefreshError::Transport)?;
            response
                .validate(self.limits.max_id_bytes)
                .map_err(|_| RefreshError::InvalidResponse)?;
            let (revision, page, next) = match response.outcome {
                Outcome::Page {
                    revision,
                    entries,
                    next,
                } => (revision, entries, next),
                Outcome::Refused {
                    failure: Failure::StaleCursor { .. },
                } => return Err(RefreshError::ResyncRequired),
                Outcome::Refused { .. } => return Err(RefreshError::Refused),
                _ => return Err(RefreshError::InvalidResponse),
            };
            if matches!(revision, Revision::Exhausted)
                || observed
                    .as_ref()
                    .is_some_and(|prior| !same_revision(prior, &revision))
            {
                return Err(RefreshError::ResyncRequired);
            }
            if page.len() > self.limits.page_size
                || page.len() > self.limits.max_entries.saturating_sub(entries.len())
            {
                return Err(RefreshError::Limit);
            }
            let mut previous = entries.last().map(|entry| entry.reference.task.as_str());
            for entry in &page {
                if previous.is_some_and(|prior| prior >= entry.reference.task.as_str()) {
                    return Err(RefreshError::InvalidResponse);
                }
                previous = Some(&entry.reference.task);
            }
            if let Some(next) = &next {
                let last = page.last().ok_or(RefreshError::InvalidResponse)?;
                if !same_revision(&next.revision, &revision)
                    || next.after.task != last.reference.task
                    || next.after.incarnation != last.reference.incarnation
                {
                    return Err(RefreshError::InvalidResponse);
                }
            }
            entries
                .try_reserve(page.len())
                .map_err(|_| RefreshError::Capacity)?;
            entries.extend(page);
            observed = Some(revision);
            if next.is_none() {
                let response = client
                    .request(Request {
                        version: 1,
                        command: Command::Revision {},
                    })
                    .map_err(RefreshError::Transport)?;
                response
                    .validate(self.limits.max_id_bytes)
                    .map_err(|_| RefreshError::InvalidResponse)?;
                let final_revision = match response.outcome {
                    Outcome::Revision { revision } => revision,
                    Outcome::Refused { .. } => return Err(RefreshError::Refused),
                    _ => return Err(RefreshError::InvalidResponse),
                };
                if !same_revision(observed.as_ref().expect("page revision"), &final_revision) {
                    return Err(RefreshError::ResyncRequired);
                }
                return Ok(View {
                    revision: final_revision,
                    entries,
                });
            }
            cursor = next;
        }
        Err(RefreshError::Limit)
    }
}
fn same_revision(left: &Revision, right: &Revision) -> bool {
    matches!((left,right),(Revision::Finite(left),Revision::Finite(right)) if left==right)
}
