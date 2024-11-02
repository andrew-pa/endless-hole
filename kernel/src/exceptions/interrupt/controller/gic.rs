//! Driver for ARM Generic Interrupt Controller.

use kernel_core::exceptions::interrupt::*;
use log::{debug, trace};
use spin::Mutex;

pub struct Generic {
    distributor_base: Mutex<*mut u32>,
    cpu_base: *mut u32,
}

/// SAFETY: The GIC CPU registers which are unprotected are actually unique for each core, so they
/// do not need to be synchronized.
unsafe impl Sync for Generic {}

fn id_to_bit_offset(id: Id) -> (usize, u32) {
    ((id / 32) as usize, (id % 32))
}

/// Set the bit flag of `register` for the interrupt `id` high.
unsafe fn write_bit_for_id(interface: *mut u32, register: usize, id: Id) {
    let (word_offset, bit_offset) = id_to_bit_offset(id);
    let ptr = interface.add(register).add(word_offset);
    trace!(
        "writing GIC register bit 0x{:x} for id={id} (byte=0x{word_offset:x}, bit={bit_offset})",
        ptr as usize
    );
    ptr.write_volatile(1 << bit_offset);
}

/// Set the byte of `register` for interrupt `id`.
unsafe fn write_byte_for_id(interface: *mut u32, register: usize, id: Id, value: u8) {
    trace!("writing GIC register byte {interface:x?}+{register:x} for id={id}, value={value:x}");
    interface
        .add(register)
        .cast::<u8>()
        .add(id as usize)
        .write_volatile(value);
}

impl Controller for Generic {
    fn configure(&self, id: Id, config: &Config) {
        debug!("configuring interrupt {id} {config:?}");
        let distributor_base = self.distributor_base.lock();
        unsafe {
            write_byte_for_id(
                *distributor_base,
                dist_regs::IPRIORITYR_N,
                id,
                config.priority,
            );
            write_byte_for_id(
                *distributor_base,
                dist_regs::ITARGETSR_N,
                id,
                config.target_cpu,
            );
        }
    }

    fn enable(&self, id: Id) {
        debug!("enable interrupt {id}");
        let distributor_base = self.distributor_base.lock();
        unsafe {
            write_bit_for_id(*distributor_base, dist_regs::ISENABLER_N, id);
        }
    }

    fn clear_pending(&self, id: Id) {
        let distributor_base = self.distributor_base.lock();
        unsafe {
            write_bit_for_id(*distributor_base, dist_regs::ICPENDR_N, id);
        }
    }

    fn disable(&self, id: Id) {
        debug!("disable interrupt {id}");
        let distributor_base = self.distributor_base.lock();
        unsafe {
            write_bit_for_id(*distributor_base, dist_regs::ICENABLER_N, id);
        }
    }

    fn ack_interrupt(&self) -> Option<Id> {
        let id = unsafe { self.cpu_base.add(cpu_regs::IAR).read_volatile() };
        trace!("ack interrupt {id}");
        if id == INTID_NONE_PENDING {
            None
        } else {
            Some(id)
        }
    }

    fn finish_interrupt(&self, id: Id) {
        trace!("finish interrupt {id}");
        unsafe {
            self.cpu_base.add(cpu_regs::EOIR).write_volatile(id);
            self.cpu_base.add(cpu_regs::DIR).write_volatile(id);
        }
    }
}

/// Register offsets for the GIC distributor (relative to its base address, by u32s).
///
/// In the specification, these are named `GICD_*`.
#[allow(unused, missing_docs)]
mod dist_regs {
    pub const CTLR: usize = 0x0000 >> 2;
    pub const TYPER: usize = 0x0004 >> 2;
    pub const STATUSR: usize = 0x0010 >> 2;
    pub const SETSPI_NSR: usize = 0x0040 >> 2;
    pub const CLRSPI_NSR: usize = 0x0048 >> 2;
    pub const SETSPI_SR: usize = 0x0050 >> 2;
    pub const CLRSPI_SR: usize = 0x0058 >> 2;
    pub const IGROUPR_N: usize = 0x0080 >> 2;
    pub const ISENABLER_N: usize = 0x0100 >> 2;
    pub const ICENABLER_N: usize = 0x0180 >> 2;
    pub const ISPENDR_N: usize = 0x0200 >> 2;
    pub const ICPENDR_N: usize = 0x0280 >> 2;
    pub const ISACTIVER_N: usize = 0x0300 >> 2;
    pub const ICACTIVER_N: usize = 0x0380 >> 2;
    pub const IPRIORITYR_N: usize = 0x0400 >> 2;
    pub const ITARGETSR_N: usize = 0x0800 >> 2;
    pub const ICFGR_N: usize = 0x0c00 >> 2;
    pub const IGRPMOD_N: usize = 0x0d00 >> 2;
    pub const SGIR: usize = 0x0f00 >> 2;
    pub const CPENDSGIR_N: usize = 0x0f10 >> 2;
    pub const SPENDSGIR_N: usize = 0x0f20 >> 2;
    pub const INMIR: usize = 0x0f80 >> 2;
}

/// Register offsets for the GIC CPU interface (relative to its base address, by u32s).
///
/// In the specification, these are named `GICC_*`.
#[allow(unused, missing_docs)]
mod cpu_regs {
    pub const CTLR: usize = 0x000 >> 2;
    pub const PMR: usize = 0x0004 >> 2;
    pub const BPR: usize = 0x008 >> 2;
    pub const IAR: usize = 0x000c >> 2;
    pub const EOIR: usize = 0x0010 >> 2;
    pub const RPR: usize = 0x0014 >> 2;
    pub const HPPIR: usize = 0x0018 >> 2;
    pub const ABPR: usize = 0x001c >> 2;
    pub const AIAR: usize = 0x0020 >> 2;
    pub const AEOIR: usize = 0x0024 >> 2;
    pub const AHPPIR: usize = 0x0028 >> 2;
    pub const STATUSR: usize = 0x002c >> 2;
    pub const APR_N: usize = 0x00d0 >> 2;
    pub const NSAPR_N: usize = 0x00e0 >> 2;
    pub const IIDR: usize = 0x00fc >> 2;
    pub const DIR: usize = 0x1000 >> 2;
}

/// Interupt ID that represents no interrupt pending.
const INTID_NONE_PENDING: Id = 1023;
