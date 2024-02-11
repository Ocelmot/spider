use serde::{Deserialize, Serialize};

/// A UiInput represents user input from a UiPage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UiInput {
    /// The user has pressed a button
    Click,
    /// The user has entered text in a textbox
    Text(String),
}
