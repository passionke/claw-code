//! AG-UI bridge: `OpenCode` Web ↔ `http-gateway-rs`. Author: kejiqing
#![allow(clippy::too_many_arguments, clippy::must_use_candidate)]

pub mod agui_events;
pub mod gateway_client;
pub mod server;

pub use agui_events::{AgUiEvent, RunAgentInput};
pub use server::serve;
