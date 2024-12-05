//! Processes (and threads).

pub mod thread;
pub use thread::Id as ThreadId;

/// A unique id for a process.
pub type Id = crate::collections::Handle;

/// A user-space process.
pub struct Process {
    /// The id of this process.
    pub id: Id,
}
