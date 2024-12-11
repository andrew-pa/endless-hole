//! Processes (and threads).
use alloc::{sync::Arc, vec::Vec};

pub mod thread;
use spin::{Mutex, RwLock};
pub use thread::{Id as ThreadId, Thread};

use crate::memory::PageTables;

/// A unique id for a process.
pub type Id = crate::collections::Handle;

/// A user-space process.
pub struct Process {
    /// The id of this process.
    pub id: Id,

    /// The supervisor process for this process.
    pub supervisor: Arc<Process>,

    /// The threads running in this process.
    pub threads: RwLock<Vec<Arc<Thread>>>,

    /// The page tables that map this process' virtual memory space.
    pub page_tables: Mutex<PageTables<'static>>,

    /// True if this process has driver-level access to the kernel.
    pub is_driver: bool,
    /// True if this process is privileged (can send messages outside of its supervisor).
    pub is_privileged: bool,
    /// True if this process is a supervisor.
    /// Child processes spawned by this process will have it as their supervisor, rather than inheriting this process' supervisor.
    pub is_supervisor: bool,
}
