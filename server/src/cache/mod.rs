pub mod accounts;
pub mod archive;
pub mod attachments;
pub mod db;
pub mod delete_queue;
pub mod fingerprint;
pub mod learning;
pub mod messages;
pub mod settings;
pub mod sync_state;
pub mod snippets;
pub mod topic;

#[cfg(test)]
mod integration_tests;
