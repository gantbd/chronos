//! Bounded V9 resynchronization over authoritative workflow status. Every
//! page is revision-bound; partial observations are discarded on any refusal.
use crate::WorkflowWireClientV9;
use chronos_protocol::{Cursor, Limits, Revision, workflow_v9 as v9};

#[derive(Clone, Copy, Debug)]
pub struct ObserverLimits {
    pub page_size: usize,
    pub max_pages: usize,
    pub max_entries: usize,
    pub max_id_bytes: usize,
}

#[derive(Clone, Debug)]
pub struct View {
    pub revision: Revision,
    pub entries: Vec<v9::Entry>,
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
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
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

pub struct Observer {
    limits: ObserverLimits,
    view: Option<View>,
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
        Ok(Self { limits, view: None })
    }

    pub fn view(&self) -> Option<&View> {
        self.view.as_ref()
    }

    pub fn invalidate(&mut self) {
        self.view = None;
    }

    pub fn refresh<C: WorkflowWireClientV9>(
        &mut self,
        client: &mut C,
    ) -> Result<(), RefreshError<C::Error>> {
        self.invalidate();
        let view = self.collect(client)?;
        self.view = Some(view);
        Ok(())
    }

    fn collect<C: WorkflowWireClientV9>(
        &self,
        client: &mut C,
    ) -> Result<View, RefreshError<C::Error>> {
        let limits = Limits {
            max_frame_bytes: usize::MAX,
            max_id_bytes: self.limits.max_id_bytes,
            max_graph_tasks: 1,
            max_dependencies_per_task: 1,
            max_capabilities_per_task: 1,
            max_capability_bytes: 1,
            max_page_entries: self.limits.page_size,
        };
        let mut entries = Vec::new();
        let mut cursor: Option<Cursor> = None;
        let mut observed: Option<Revision> = None;
        for _ in 0..self.limits.max_pages {
            let request = v9::Request {
                version: 9,
                command: v9::Command::Page {
                    cursor: cursor.clone(),
                    limit: self.limits.page_size,
                },
            };
            let response = client
                .request_workflow_v9(request.clone())
                .map_err(RefreshError::Transport)?;
            response
                .validate_for(&request, limits)
                .map_err(|_| RefreshError::InvalidResponse)?;
            let (revision, page, next) = match response.outcome {
                v9::Outcome::Page {
                    revision,
                    entries,
                    next,
                } => (revision, entries, next),
                v9::Outcome::Refused {
                    code: v9::RefusalCode::StaleCursor,
                } => return Err(RefreshError::ResyncRequired),
                v9::Outcome::Refused {
                    code: v9::RefusalCode::Limit | v9::RefusalCode::InvalidPageSize,
                } => return Err(RefreshError::Limit),
                v9::Outcome::Refused {
                    code: v9::RefusalCode::Capacity,
                } => return Err(RefreshError::Capacity),
                v9::Outcome::Refused { .. } => return Err(RefreshError::Refused),
                _ => return Err(RefreshError::InvalidResponse),
            };
            if observed
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
            let mut previous = entries
                .last()
                .map(|entry: &v9::Entry| entry.reference.task.as_str());
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
                return Ok(View {
                    revision: observed.expect("page response always supplies a revision"),
                    entries,
                });
            }
            cursor = next;
        }
        Err(RefreshError::Limit)
    }
}

fn same_revision(left: &Revision, right: &Revision) -> bool {
    matches!((left, right), (Revision::Finite(left), Revision::Finite(right)) if left == right)
        || matches!((left, right), (Revision::Exhausted, Revision::Exhausted))
}
