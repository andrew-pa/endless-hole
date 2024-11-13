//! Interrupts from hardware devices.
use alloc::boxed::Box;
use kernel_core::{
    exceptions::{interrupt::Handler, InterruptController},
    platform::device_tree::DeviceTree,
};
use log::{info, trace};
use spin::once::Once;

use crate::timer::Timer;

pub mod controller;

/// The global interrupt handler policy.
pub static HANDLER_POLICY: Once<Handler<'static, 'static, Timer>> = Once::new();

/// The current interrupt controller device in the system.
pub static CONTROLLER: Once<Box<dyn InterruptController + Send + Sync>> = Once::new();

/// The global instance of the system timer interface.
pub static TIMER: Once<Timer> = Once::new();

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

    controller.global_initialize();

    let timer_node = device_tree
        .iter_node_properties(b"/timer")
        .expect("have timer node");

    let timer = TIMER.call_once(|| {
        Timer::in_device_tree(timer_node, controller.as_ref(), 10).expect("configure system timer")
    });

    HANDLER_POLICY.call_once(|| Handler::new(controller.as_ref(), timer));

    init_for_core();

    info!("Interrupts initialized!");
}

/// Perform initialization for interrupts that needs to happen for each core on the system.
pub fn init_for_core() {
    let ctrl = CONTROLLER.get().unwrap();
    ctrl.initialize_for_core();
    TIMER.get().unwrap().start_for_core(ctrl.as_ref());
}

/// Wait for an interrupt to occur, pausing execution.
#[inline]
pub fn wait_for_interrupt() {
    unsafe {
        core::arch::asm!("wfi");
    }
}
