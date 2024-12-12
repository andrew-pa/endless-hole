//! Thread switching mechanism.

use alloc::vec::Vec;
use kernel_core::{
    collections::HandleMap,
    memory::{AddressSpaceIdPool, VirtualAddress},
    platform::cpu::{CoreInfo, CpuIdReader, Id as CpuId},
    process::{
        thread::{
            scheduler::RoundRobinScheduler, ProcessorState, Registers, SavedProgramStatus,
            Scheduler, State, Thread, MAX_THREAD_ID,
        },
        Process,
    },
};
use log::{debug, info, trace};
use spin::once::Once;

use crate::memory::switch_el0_context;

/// Implementation of [`CpuIdReader`] that reads the real system registers.
pub struct SystemCpuIdReader;

impl CpuIdReader for SystemCpuIdReader {
    fn current_cpu() -> CpuId {
        let mut core_id: usize;
        unsafe {
            core::arch::asm!(
                "mrs {core_id}, MPIDR_EL1",
                core_id = out(reg) core_id
            );
        }
        // clear multiprocessor flag bit in MPIDR register
        core_id & !0x8000_0000
    }
}

pub type PlatformScheduler = RoundRobinScheduler<SystemCpuIdReader>;

pub static SCHEDULER: Once<PlatformScheduler> = Once::new();
pub static THREADS: Once<HandleMap<Thread>> = Once::new();
pub static PROCESSES: Once<HandleMap<Process>> = Once::new();
static ASID_POOL: Once<AddressSpaceIdPool> = Once::new();

pub fn init(cores: &[CoreInfo]) {
    debug!("Initalizing threads...");

    let threads = THREADS.call_once(|| HandleMap::new(MAX_THREAD_ID));

    trace!("Creating thread scheduler...");

    let init_threads: Vec<_> = cores
        .iter()
        .map(|info| {
            let idle_thread = Thread::new(threads, None, State::Running, unsafe {
                ProcessorState::new_for_idle_thread()
            });
            (info.id, idle_thread)
        })
        .collect();

    SCHEDULER.call_once(|| PlatformScheduler::new(&init_threads));

    ASID_POOL.call_once(AddressSpaceIdPool::default);

    info!("Threads initialized!");
}

/// Read the current value of the `SPSR_EL1` register.
pub fn read_saved_program_status() -> SavedProgramStatus {
    let mut v: u64;
    unsafe {
        core::arch::asm!("mrs {v}, SPSR_EL1", v = out(reg) v);
    }
    SavedProgramStatus(v)
}

/// Write to the `SPSR_EL1` register.
///
/// # Safety
/// It is up to the caller to ensure that the `SavedProgramStatus` value is correct.
pub unsafe fn write_saved_program_status(spsr: &SavedProgramStatus) {
    core::arch::asm!("msr SPSR_EL1, {v}", v = in(reg) spsr.0);
}

/// Read the value of the program counter when the exception occured.
pub fn read_exception_link_reg() -> VirtualAddress {
    let mut v: usize;
    unsafe {
        core::arch::asm!("mrs {v}, ELR_EL1", v = out(reg) v);
    }
    v.into()
}

/// Write the value that the program counter will assume when the exception handler is finished.
///
/// # Safety
/// It is up to the caller to ensure that the address is valid to store as the program counter.
pub unsafe fn write_exception_link_reg(addr: VirtualAddress) {
    core::arch::asm!("msr ELR_EL1, {v}", v = in(reg) usize::from(addr));
}

/// Reads the stack pointer for exception level `el`.
pub fn read_stack_pointer(el: u8) -> VirtualAddress {
    let mut v: usize;
    unsafe {
        match el {
            0 => core::arch::asm!("mrs {v}, SP_EL0", v = out(reg) v),
            1 => core::arch::asm!("mrs {v}, SP_EL1", v = out(reg) v),
            2 => core::arch::asm!("mrs {v}, SP_EL2", v = out(reg) v),
            // 3 => core::arch::asm!("mrs {v}, SP_EL3", v = out(reg) v),
            _ => panic!("invalid exception level {el}"),
        }
    }
    v.into()
}

/// Writes the stack pointer for exception level `el`.
///
/// # Safety
/// It is up to the caller to ensure that the pointer is valid to be stack pointer (i.e. the memory
/// is allocated and mapped correctly). It is also up to the caller to pass a value for `el` that
/// is valid considering the current value of `el`.
pub unsafe fn write_stack_pointer(el: u8, sp: VirtualAddress) {
    let addr = usize::from(sp);
    match el {
        0 => core::arch::asm!("msr SP_EL0, {v}", v = in(reg) addr),
        1 => core::arch::asm!("msr SP_EL1, {v}", v = in(reg) addr),
        2 => core::arch::asm!("msr SP_EL2, {v}", v = in(reg) addr),
        // 3 => core::arch::asm!("msr SP_EL3, {v}", v = in(reg) sp.0),
        _ => panic!("invalid exception level {el}"),
    }
}

pub unsafe fn save_current_thread_state(registers: &Registers) {
    // Determine the current running thread according to the scheduler.
    // We assume that this thread is the one currently executing on this processor.
    let current_thread = SCHEDULER
        .get()
        .expect("scheduler init before thread switch")
        .current_thread();

    // Save the processor state into the thread object.
    let mut s = current_thread
        .processor_state
        .try_lock()
        .expect("no locks on current thread's execution state");
    s.spsr = read_saved_program_status();
    s.program_counter = read_exception_link_reg();
    s.stack_pointer = read_stack_pointer(0);
    s.registers = *registers;

    trace!(
        "saving processor state to thread#{}, pc={:?}",
        current_thread.id,
        s.program_counter
    );
}

pub unsafe fn restore_current_thread_state(registers: &mut Registers) {
    // Determine the current running thread according to the scheduler.
    let current_thread = SCHEDULER
        .get()
        .expect("scheduler init before thread switch")
        .current_thread();

    // Switch to this thread's process' page table, if it is a user space thread.
    if let Some(process) = current_thread.parent.as_ref() {
        let (asid, full_flush) = process.get_address_space_id(ASID_POOL.get().unwrap());
        let pt = process.page_tables.read();
        switch_el0_context(&pt, asid, full_flush);
    }

    // Restore the state of the processor.
    let s = current_thread
        .processor_state
        .try_lock()
        .expect("no locks on current thread's execution state");
    *registers = s.registers;
    write_stack_pointer(0, s.stack_pointer);
    write_exception_link_reg(s.program_counter);
    write_saved_program_status(&s.spsr);

    trace!(
        "restoring processor state to thread#{}, pc={:?}",
        current_thread.id,
        s.program_counter
    );
}
