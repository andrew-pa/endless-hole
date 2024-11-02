//! Interrupts from hardware devices.
use kernel_core::{exceptions::interrupt::Handler, platform::device_tree::DeviceTree};
use log::info;
use spin::once::Once;

pub mod controller;

/// The global interrupt handler policy.
pub static HANDLER_POLICY: Once<Handler<'static>> = Once::new();

/// Initialize the interrupt controller and interrupt handler.
pub fn init(device_tree: &DeviceTree<'_>) {
    HANDLER_POLICY.call_once(|| todo!());

    info!("Interrupts initialized!");
}
