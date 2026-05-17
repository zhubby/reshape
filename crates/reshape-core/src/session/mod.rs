pub mod store;

use crate::error::{ReshapeError, Result};
use crate::protocol::{DEFAULT_SESSION_KEY, InputEvent, OutputEvent};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub session_key: String,
    pub turn_index: u64,
    pub history: Vec<SessionMessage>,
    pub recent_artifacts: Vec<String>,
    pub budget: BudgetUsage,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SessionMessage {
    Input(InputEvent),
    Output(OutputEvent),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetUsage {
    pub tool_iterations: usize,
    pub tool_calls: usize,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            session_key: DEFAULT_SESSION_KEY.to_string(),
            turn_index: 0,
            history: Vec::new(),
            recent_artifacts: Vec::new(),
            budget: BudgetUsage::default(),
        }
    }
}

impl Session {
    pub fn record_input(&mut self, event: InputEvent) {
        self.turn_index += 1;
        self.history.push(SessionMessage::Input(event));
    }

    pub fn record_output(&mut self, event: OutputEvent) {
        self.history.push(SessionMessage::Output(event));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnState {
    Received,
    Validating,
    Executing,
    Publishing,
    Completed,
    Degraded,
    Failed,
}

impl TurnState {
    pub fn transition_to(self, next: TurnState) -> Result<TurnState> {
        let valid = matches!(
            (self, next),
            (TurnState::Received, TurnState::Validating)
                | (TurnState::Validating, TurnState::Executing)
                | (TurnState::Validating, TurnState::Failed)
                | (TurnState::Executing, TurnState::Publishing)
                | (TurnState::Executing, TurnState::Degraded)
                | (TurnState::Executing, TurnState::Failed)
                | (TurnState::Publishing, TurnState::Completed)
        );

        if valid {
            Ok(next)
        } else {
            Err(ReshapeError::InvalidStateTransition {
                from: format!("{self:?}"),
                to: format!("{next:?}"),
            })
        }
    }
}
