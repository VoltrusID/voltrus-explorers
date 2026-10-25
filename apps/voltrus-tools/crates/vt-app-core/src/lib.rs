//! The app model shared by every front end, with no UI: saved connections,
//! tabs, how panes split inside a tab, and where it is all stored.

pub mod forms;
mod layout;
pub mod search;
pub mod store;
mod workspace;

pub use layout::{Axis, Branch, Layout};
pub use workspace::{
    Appearance, Connection, ConnectionId, Pane, PaneId, PaneView, Tab, TabId, UiState, Workspace,
};
