//! Sync — bidirectional local-store ⇄ Sheets reconciliation.
//! `merge` is the pure decision core; `engine` executes its plan
//! against a `SyncRemote`.

mod engine;
mod merge;

pub use engine::{sync_views, SyncRemote, ViewSyncResult};
pub use merge::{merge, Action, MergePlan, RemoteRow};
