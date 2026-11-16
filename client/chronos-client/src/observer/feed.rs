//! One-shot explicit feed pulls and bounded full resynchronization. No timer,
//! background worker, mutation retry or automatic gap-recovery loop.
use super::{Observer, RefreshError, View, increment, same_revision};
use crate::{WireClientV1, WireClientV3};
use chronos_protocol::{Limits, coordination_v3 as wire};
#[derive(Clone, Copy, Debug, Default)]
pub struct FeedSignals {
    pub resyncs: u64,
    pub polls: u64,
    pub change_sets: u64,
    pub changes: u64,
    pub failures: u64,
    pub gaps: u64,
    /// Resync includes its snapshot and final pull; poll is that pull's subpath.
    pub resync_time: std::time::Duration,
    pub poll_time: std::time::Duration,
    pub saturated: bool,
}
fn add_time(value: &mut std::time::Duration, delta: std::time::Duration, saturated: &mut bool) {
    if let Some(total) = value.checked_add(delta) {
        *value = total;
    } else {
        *value = std::time::Duration::MAX;
        *saturated = true;
    }
}
impl Observer {
    pub fn feed_cursor(&self) -> Option<&wire::FeedCursor> {
        self.feed_cursor.as_ref()
    }
    pub fn feed_signals(&self) -> FeedSignals {
        self.feed_signals
    }
    /// One bounded full snapshot plus a generation-checked pull. At most
    /// max_pages + 3 requests; `true` means another explicit pull is needed.
    /// A view is revision-consistent, not a continuing authority/currentness
    /// promise. Failure invalidates the view; callers own any retained copies.
    pub fn resync_feed<C>(
        &mut self,
        client: &mut C,
    ) -> Result<bool, RefreshError<<C as WireClientV1>::Error>>
    where
        C: WireClientV1 + WireClientV3<Error = <C as WireClientV1>::Error>,
    {
        self.invalidate();
        let started = std::time::Instant::now();
        increment(
            &mut self.feed_signals.resyncs,
            &mut self.feed_signals.saturated,
        );
        let result = (|| {
            let response = client
                .request_v3(wire::Request {
                    version: 3,
                    command: wire::Command::Observe {
                        after: None,
                        limit: self.limits.page_size,
                    },
                })
                .map_err(RefreshError::Transport)?;
            self.validate_feed(&response)?;
            let mut head = match response.outcome {
                wire::Outcome::ResyncRequired { head } => head,
                wire::Outcome::Refused { .. } => return Err(RefreshError::Refused),
                _ => return Err(RefreshError::InvalidResponse),
            };
            self.refresh(client)?;
            head.revision = self
                .view
                .as_ref()
                .ok_or(RefreshError::InvalidResponse)?
                .revision
                .clone();
            self.feed_cursor = Some(head);
            self.poll_changes(client)
        })();
        if result.is_err() {
            self.invalidate();
        }
        add_time(
            &mut self.feed_signals.resync_time,
            started.elapsed(),
            &mut self.feed_signals.saturated,
        );
        result
    }
    /// Fetch/apply one bounded page of committed deltas atomically to this
    /// client's view. Gaps/refusals/disconnects never trigger implicit resync.
    pub fn poll_changes<C: WireClientV3>(
        &mut self,
        client: &mut C,
    ) -> Result<bool, RefreshError<C::Error>> {
        let started = std::time::Instant::now();
        increment(
            &mut self.feed_signals.polls,
            &mut self.feed_signals.saturated,
        );
        let result = (|| {
            let after = self
                .feed_cursor
                .clone()
                .ok_or(RefreshError::ResyncRequired)?;
            let response = client
                .request_v3(wire::Request {
                    version: 3,
                    command: wire::Command::Observe {
                        after: Some(after),
                        limit: self.limits.page_size,
                    },
                })
                .map_err(RefreshError::Transport)?;
            self.validate_feed(&response)?;
            match response.outcome {
                wire::Outcome::ResyncRequired { .. } => Err(RefreshError::ResyncRequired),
                wire::Outcome::Refused { .. } => Err(RefreshError::Refused),
                wire::Outcome::Changes {
                    cursor,
                    changes,
                    more,
                } => self.apply_changes(cursor, changes, more),
                _ => Err(RefreshError::InvalidResponse),
            }
        })();
        if result.is_err() {
            self.invalidate();
            increment(
                &mut self.feed_signals.failures,
                &mut self.feed_signals.saturated,
            );
            if matches!(&result, Err(RefreshError::ResyncRequired)) {
                increment(
                    &mut self.feed_signals.gaps,
                    &mut self.feed_signals.saturated,
                );
            }
        }
        add_time(
            &mut self.feed_signals.poll_time,
            started.elapsed(),
            &mut self.feed_signals.saturated,
        );
        result
    }
    fn validate_feed<E>(&self, response: &wire::Response) -> Result<(), RefreshError<E>> {
        let max_changes = self
            .limits
            .max_entries
            .checked_mul(2)
            .ok_or(RefreshError::Limit)?;
        // Owned responses were allocated by the client port. Framing/RSS limits
        // remain the producer's responsibility, just as for V1 snapshot reads.
        response
            .validate(Limits {
                max_frame_bytes: usize::MAX,
                max_id_bytes: self.limits.max_id_bytes,
                max_graph_tasks: max_changes,
                max_dependencies_per_task: 0,
                max_capabilities_per_task: 0,
                max_capability_bytes: 0,
                max_page_entries: self.limits.page_size,
            })
            .map_err(|error| match error {
                chronos_protocol::CodecError::Limit => RefreshError::Limit,
                chronos_protocol::CodecError::Capacity => RefreshError::Capacity,
                _ => RefreshError::InvalidResponse,
            })
    }
    fn apply_changes<E>(
        &mut self,
        cursor: wire::FeedCursor,
        changes: Vec<wire::ChangeSet>,
        more: bool,
    ) -> Result<bool, RefreshError<E>> {
        let previous = self
            .feed_cursor
            .as_ref()
            .ok_or(RefreshError::ResyncRequired)?;
        let view = self.view.as_ref().ok_or(RefreshError::ResyncRequired)?;
        if cursor.generation != previous.generation {
            return Err(RefreshError::ResyncRequired);
        }
        if !same_revision(&previous.revision, &view.revision) {
            return Err(RefreshError::InvalidResponse);
        }
        if changes.is_empty() {
            if more || !same_revision(&view.revision, &cursor.revision) {
                return Err(RefreshError::InvalidResponse);
            }
            self.feed_cursor = Some(cursor);
            return Ok(false);
        }
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(view.entries.len())
            .map_err(|_| RefreshError::Capacity)?;
        entries.extend(view.entries.iter().cloned());
        let mut revision = view.revision.clone();
        let mut sets = 0_u64;
        let mut applied = 0_u64;
        for set in changes {
            if !same_revision(&set.from, &revision) {
                return Err(RefreshError::InvalidResponse);
            }
            increment(&mut sets, &mut self.feed_signals.saturated);
            for change in set.changes {
                increment(&mut applied, &mut self.feed_signals.saturated);
                match change {
                    wire::TaskChange::Upsert { entry } => {
                        match entries.binary_search_by(|existing| {
                            existing.reference.task.cmp(&entry.reference.task)
                        }) {
                            Ok(index) => {
                                if entries[index].reference.incarnation
                                    != entry.reference.incarnation
                                {
                                    return Err(RefreshError::InvalidResponse);
                                }
                                entries[index] = entry;
                            }
                            Err(index) => {
                                if entries.len() >= self.limits.max_entries {
                                    return Err(RefreshError::Limit);
                                }
                                entries.try_reserve(1).map_err(|_| RefreshError::Capacity)?;
                                entries.insert(index, entry);
                            }
                        }
                    }
                    wire::TaskChange::Removed { reference } => {
                        let index = entries
                            .binary_search_by(|entry| entry.reference.task.cmp(&reference.task))
                            .map_err(|_| RefreshError::InvalidResponse)?;
                        if entries[index].reference.incarnation != reference.incarnation {
                            return Err(RefreshError::InvalidResponse);
                        }
                        entries.remove(index);
                    }
                }
            }
            revision = set.to;
        }
        if !same_revision(&revision, &cursor.revision) {
            return Err(RefreshError::InvalidResponse);
        }
        self.view = Some(View { revision, entries });
        self.feed_cursor = Some(cursor);
        if let Some(total) = self.feed_signals.change_sets.checked_add(sets) {
            self.feed_signals.change_sets = total;
        } else {
            self.feed_signals.change_sets = u64::MAX;
            self.feed_signals.saturated = true;
        }
        if let Some(total) = self.feed_signals.changes.checked_add(applied) {
            self.feed_signals.changes = total;
        } else {
            self.feed_signals.changes = u64::MAX;
            self.feed_signals.saturated = true;
        }
        Ok(more)
    }
}
