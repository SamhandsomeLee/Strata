//! Session state (design §2.4).
//!
//! Flat message history shared across all loop turns (design §3). Serializable
//! `history` + `turn` define the checkpoint boundary (decision 5); save/load I/O is M5+.

use serde::{Deserialize, Serialize};

use crate::message::Message;

/// Agent loop mutable state: one linear history and a tool-round counter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub history: Vec<Message>,
    pub turn: u32,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub fn new() -> Self {
        Self {
            history: Vec::new(),
            turn: 0,
        }
    }

    pub fn with_history(history: Vec<Message>) -> Self {
        Self { history, turn: 0 }
    }

    pub fn push(&mut self, message: Message) {
        self.history.push(message);
    }

    pub fn is_empty(&self) -> bool {
        self.history.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::ContentBlock;

    #[test]
    fn new_session_starts_empty_at_turn_zero() {
        let session = Session::new();
        assert!(session.is_empty());
        assert_eq!(session.turn, 0);
        assert_eq!(Session::default().turn, 0);
    }

    #[test]
    fn with_history_and_push_preserves_order() {
        let mut session = Session::with_history(vec![Message::system("you are helpful")]);
        session.push(Message::user("hi"));
        assert_eq!(session.history.len(), 2);
        assert_eq!(session.history[0].role, crate::message::Role::System);
        assert_eq!(session.history[1].text(), "hi");
    }

    #[test]
    fn session_round_trip_json() {
        let session = Session {
            history: vec![
                Message::user("question"),
                Message::assistant(vec![ContentBlock::Text("answer".into())]),
                Message::tool(ContentBlock::ToolResult {
                    id: "call_1".into(),
                    content: "42".into(),
                    is_error: false,
                }),
            ],
            turn: 3,
        };

        let json = serde_json::to_string(&session).expect("serialize");
        let restored: Session = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(session, restored);
        assert_eq!(restored.turn, 3);
    }
}
