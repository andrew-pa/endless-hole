//! Standard system timer driver.

use bitfield::bitfield;
use core::arch::asm;
use kernel_core::{
    exceptions::{interrupt, InterruptController, InterruptId},
    platform::device_tree::{
        iter::{NodeItem, NodePropertyIter},
        ParseError, PropertyNotFoundSnafu, UnexpectedValueSnafu, Value,
    },
};
use log::debug;
use snafu::{ensure, OptionExt};

/// Read the compare value register (`CNTP_CVAL_EL0`).
pub fn read_compare_value() -> u64 {
    let mut cv: u64;
    unsafe {
        asm!("mrs {cv}, CNTP_CVAL_EL0", cv = out(reg) cv);
    }
    cv
}

/// Write the compare value register (`CNTP_CVAL_EL0`).
pub fn write_compare_value(compare_value: u64) {
    unsafe {
        asm!("msr CNTP_CVAL_EL0, {cv}", cv = in(reg) compare_value);
    }
}

/// Read timer value register (`CNTP_TVAL_EL0`).
pub fn read_timer_value() -> u32 {
    let mut tv: u64;
    unsafe {
        asm!("mrs {tv}, CNTP_TVAL_EL0", tv = out(reg) tv);
    }
    tv as u32
}

/// Write timer value register (`CNTP_TVAL_EL0`).
pub fn write_timer_value(timer_value: u32) {
    unsafe {
        asm!("msr CNTP_TVAL_EL0, {cv:x}", cv = in(reg) timer_value);
    }
}

/// Read timer counter register (`CNTPCT_EL0`).
pub fn counter() -> u64 {
    let mut cntpct: u64;
    unsafe {
        asm!("mrs {val}, CNTPCT_EL0", val = out(reg) cntpct);
    }
    cntpct
}

/// Read timer counter frequency register (`CNTFRQ_EL0`).
pub fn frequency() -> u32 {
    let mut freq: u64;
    unsafe {
        asm!("mrs {val}, CNTFRQ_EL0", val = out(reg) freq);
    }
    freq as u32
}

bitfield! {
    struct TimerControlRegister(u64);
    impl Debug;
    u8;
    istatus, _: 2;
    imask, set_imask: 1;
    enable, set_enable: 0;
}

/// Read the timer control register (`CNTP_CTL_EL0`).
fn read_control() -> TimerControlRegister {
    let mut ctrl: u64;
    unsafe {
        asm!("mrs {ctrl}, CNTP_CTL_EL0", ctrl = out(reg) ctrl);
    }
    TimerControlRegister(ctrl)
}

/// Write the timer control register (`CNTP_CTL_EL0`).
fn write_control(r: TimerControlRegister) {
    unsafe {
        asm!("msr CNTP_CTL_EL0, {ctrl}", ctrl = in(reg) r.0);
    }
}

/// Check to see if the timer condition has been met.
pub fn condition_met() -> bool {
    read_control().istatus()
}

/// Check to see if the timer interrupt is enabled.
pub fn interrupts_enabled() -> bool {
    !read_control().imask()
}

/// Enable/disable the timer interrupt.
pub fn set_interrupts_enabled(enabled: bool) {
    let mut c = read_control();
    c.set_imask(!enabled);
    write_control(c);
}

/// Check if the timer is enabled.
pub fn enabled() -> bool {
    read_control().enable()
}

/// Enable/disable the timer.
pub fn set_enabled(enabled: bool) {
    let mut c = read_control();
    c.set_enable(enabled);
    write_control(c);
}

/// A list of device tree `compatible` strings (see section 2.3.1 of the spec) that this driver is compatible with.
const COMPATIBLE: &[&[u8]] = &[b"arm,armv7-timer", b"arm,armv8-timer"];

pub fn in_device_tree<'dt>(
    node: NodePropertyIter<'dt>,
    intc: &dyn InterruptController,
) -> Result<(InterruptId, interrupt::TriggerMode), ParseError<'dt>> {
    let mut int = None;

    for (name, value) in node {
        match name {
            b"compatible" => match &value {
                Value::StringList(strings) => {
                    // make sure that the driver is compatible with the device
                    ensure!(
                        strings.iter().any(|model_name| COMPATIBLE
                            .iter()
                            .any(|supported_model_name| model_name.to_bytes()
                                == *supported_model_name)),
                        UnexpectedValueSnafu {
                            name,
                            value,
                            reason: "incompatible"
                        }
                    );
                    debug!("Timer compatible device: {strings:?}");
                }
                _ => {
                    return Err(ParseError::UnexpectedType {
                        name,
                        value,
                        expected_type: "StringList",
                    })
                }
            },
            b"interrupts" => match value {
                Value::Bytes(interrupts_blob) => {
                    let i = intc.interrupt_in_device_tree(interrupts_blob, 1).context(
                        UnexpectedValueSnafu {
                            name,
                            value,
                            reason: "expected interrupt #1 to exist",
                        },
                    )?;
                    int = Some(i);
                }
                _ => {
                    return Err(ParseError::UnexpectedType {
                        name,
                        value,
                        expected_type: "Bytes",
                    })
                }
            },
            _ => {}
        }
    }

    int.context(PropertyNotFoundSnafu { name: "interrupts" })
}
