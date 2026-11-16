//! Exact one-hop relationships visible to the current principal.
use crate::{CodecError, Limits, Reference, Revision, validate_reference, validate_revision};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Prerequisites,
    Dependents,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Neighbors {
        expected: Revision,
        task: Reference,
        direction: Direction,
        limit: usize,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u16,
    pub command: Command,
}

impl Request {
    pub fn validate(&self, limits: Limits) -> Result<(), CodecError> {
        if self.version != 6 {
            return Err(CodecError::Version);
        }
        let Command::Neighbors {
            expected,
            task,
            limit,
            ..
        } = &self.command;
        validate_revision(expected)?;
        validate_reference(task, limits.max_id_bytes)?;
        if matches!(expected, Revision::Exhausted) {
            return Err(CodecError::Malformed);
        }
        if !(1..=limits.max_page_entries).contains(limit) {
            return Err(CodecError::Limit);
        }
        Ok(())
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    Unauthorized,
    InvalidRequest,
    Limit,
    Stale,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum Outcome {
    VisibleNeighbors {
        revision: Revision,
        task: Reference,
        direction: Direction,
        neighbors: Vec<Reference>,
        truncated: bool,
    },
    Refused {
        code: Refusal,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub version: u16,
    pub outcome: Outcome,
}

impl Response {
    pub fn validate_for(&self, request: &Request, limits: Limits) -> Result<(), CodecError> {
        if self.version != 6 {
            return Err(CodecError::Version);
        }
        if let Outcome::VisibleNeighbors {
            revision,
            task,
            direction,
            neighbors,
            truncated,
        } = &self.outcome
        {
            let Command::Neighbors {
                expected,
                task: requested,
                direction: requested_direction,
                limit,
            } = &request.command;
            if !matches!((revision, expected), (Revision::Finite(a), Revision::Finite(b)) if a == b)
                || task.task != requested.task
                || task.incarnation != requested.incarnation
                || direction != requested_direction
            {
                return Err(CodecError::Malformed);
            }
            if neighbors.len() > *limit || (*truncated && neighbors.len() != *limit) {
                return Err(CodecError::Limit);
            }
            let mut previous = None;
            for neighbor in neighbors {
                validate_reference(neighbor, limits.max_id_bytes)?;
                if neighbor.task == task.task
                    || previous.is_some_and(|prior| prior >= neighbor.task.as_str())
                {
                    return Err(CodecError::Malformed);
                }
                previous = Some(neighbor.task.as_str());
            }
        }
        Ok(())
    }
}
