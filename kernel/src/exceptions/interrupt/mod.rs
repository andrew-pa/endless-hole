//! Interrupts from hardware devices.
use alloc::boxed::Box;
use kernel_core::{
    exceptions::{interrupt::Handler, InterruptController},
    platform::device_tree::DeviceTree,
};
use log::{info, trace};
use spin::once::Once;

pub mod controller;

/// The global interrupt handler policy.
pub static HANDLER_POLICY: Once<Handler<'static>> = Once::new();

/// The current interrupt controller device in the system.
pub static CONTROLLER: Once<Box<dyn InterruptController + Send + Sync>> = Once::new();

/// Initialize the interrupt controller and interrupt handler.
pub fn init(device_tree: &DeviceTree<'_>) {
    trace!("Initializing interrupts…");

    // TODO: we assume here that the interrupt controller is under `/intc@?`, which is definitely
    // not true in general! We need to either use the `/interrupt-parent` property or the
    // `interrupt-controller` marker property.
    let intc_node = device_tree
        .iter_nodes_named(b"/", b"intc")
        .expect("root node")
        .next()
        .expect("have intc node");

    let controller = CONTROLLER.call_once(|| {
        Box::new(
            controller::gic2::GenericV2::in_device_tree(intc_node.properties)
                .expect("configure interrupt controller"),
        )
    });

    HANDLER_POLICY.call_once(|| Handler::new(controller.as_ref()));

    info!("Interrupts initialized!");
}
