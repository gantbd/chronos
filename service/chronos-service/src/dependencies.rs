//! Filter every edge before including it in a bounded response.
use crate::{Authorizer, Service};
use chronos_engine::{StateRevision, TaskRefV1};
use chronos_host::dependencies::{DependencyRefusal, DependencyRequest, VisibleDependencies};

pub trait DependencyAuthorizer: Authorizer {
    fn allows_dependency_read(&mut self, request: &DependencyRequest) -> bool;
    fn allows_dependency_edge(&mut self, request: &DependencyRequest, neighbor: &TaskRefV1)
    -> bool;
}

impl<A: DependencyAuthorizer> Service<A> {
    pub fn visible_dependencies(
        &mut self,
        request: DependencyRequest,
    ) -> Result<VisibleDependencies, DependencyRefusal> {
        if !self.authorizer.allows_dependency_read(&request) {
            return Err(DependencyRefusal::Unauthorized);
        }
        if !(1..=self.limits.max_page_entries).contains(&request.limit) {
            return Err(DependencyRefusal::InvalidLimit);
        }
        let revision = self.engine.task_state_revision();
        if revision != request.expected || revision == StateRevision::Exhausted {
            return Err(DependencyRefusal::Stale);
        }
        let candidates = self
            .engine
            .dependency_neighbors(&request.task, request.direction)
            .ok_or(DependencyRefusal::Stale)?;
        let mut neighbors = Vec::new();
        let mut truncated = false;
        for neighbor in candidates {
            if self.authorizer.allows_dependency_edge(&request, &neighbor) {
                if neighbors.len() == request.limit {
                    truncated = true;
                } else {
                    neighbors.push(neighbor);
                }
            }
        }
        Ok(VisibleDependencies {
            revision,
            neighbors,
            truncated,
        })
    }
}
