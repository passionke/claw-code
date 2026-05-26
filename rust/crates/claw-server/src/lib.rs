//! Multi-tenant HTTP server for Claw Code workspaces (library + binary).

pub mod auth;
pub mod blocking_client;
pub mod chat;
pub mod crypto;
pub mod db;
pub mod error;
pub mod profiles;
pub mod server;
pub mod workspaces;

pub use error::ServerError;
pub use server::{serve, AppState};
