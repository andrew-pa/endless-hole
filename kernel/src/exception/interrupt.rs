use kernel_core::{exceptions::interrupt::*, platform::device_tree::DeviceTree};
use log::info;
use spin::once::Once;

/// The global interrupt handler policy.
pub static HANDLER_POLICY: Once<Handler<'static>> = Once::new();

/// Initialize the interrupt controller and interrupt handler.
pub fn init(device_tree: &DeviceTree<'_>) {
    HANDLER_POLICY.call_once(|| todo!());

    info!("Interrupts initialized!");
}
