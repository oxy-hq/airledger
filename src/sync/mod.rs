//! Sync — bidirectional local-store ⇄ Sheets reconciliation.
//! `merge` is the pure decision core; `engine` executes its plan
//! against a `SyncRemote`.

mod merge;

pub use merge::{merge, Action, MergePlan, RemoteRow};
