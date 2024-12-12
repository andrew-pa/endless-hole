//! Processes (and threads).
use alloc::{sync::Arc, vec::Vec};

pub mod thread;
use spin::RwLock;
pub use thread::{Id as ThreadId, Thread};

use crate::memory::{AddressSpaceId, AddressSpaceIdPool, PageTables};

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
    pub page_tables: RwLock<PageTables<'static>>,

    /// The current address space ID and its generation.
    pub address_space_id: RwLock<(Option<AddressSpaceId>, u32)>,

    /// True if this process has driver-level access to the kernel.
    pub is_driver: bool,
    /// True if this process is privileged (can send messages outside of its supervisor).
    pub is_privileged: bool,
    /// True if this process is a supervisor.
    /// Child processes spawned by this process will have it as their supervisor, rather than inheriting this process' supervisor.
    pub is_supervisor: bool,
}

impl Process {
    /// Get or allocate an address space ID for this process.
    /// Returns true if a new generation has occured.
    pub fn get_address_space_id(&self, pool: &AddressSpaceIdPool) -> (AddressSpaceId, bool) {
        loop {
            let (asid, generation) = *self.address_space_id.read();
            if generation == pool.current_generation() {
                if let Some(i) = asid {
                    // current ASID is valid
                    return (i, false);
                }
            }
            // ASID is invalid so we need to allocate and store a new one. However only one thread
            // needs to do this, so if we don't get the lock, we'll wait for the write to finish.
            if let Some(mut asid_writer) = self.address_space_id.try_write() {
                // we can write, so allocate a new ASID
                let new = pool.allocate();
                *asid_writer = (Some(new.0), new.1);
                return (new.0, new.1 != generation);
            }
        }
    }
}
