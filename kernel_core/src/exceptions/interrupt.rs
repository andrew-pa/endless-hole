//! Interrupts are exceptions caused by hardware devices.

use log::{debug, trace};

use crate::platform::timer::SystemTimer;

/// The identifier of an interrupt.
pub type Id = u32;

/// Trigger mode for an interrupt.
#[derive(Debug, Default)]
pub enum TriggerMode {
    /// Use level triggering.
    #[default]
    Level,
    /// Use edge triggering.
    Edge,
}

/// A value that gives a boolean value for each CPU in the system.
pub type CpuMask = u8;

/// The configuration of an interrupt with the interrupt controller.
#[derive(Debug, Default)]
pub struct Config {
    /// Priority level.
    pub priority: u8,
    /// CPUs that will handle the interrupt.
    pub target_cpu: CpuMask,
    /// Triggering mode for the interrupt.
    pub mode: TriggerMode,
}

/// An interrupt controller manages and collates interrupts for the processor.
/// This is the generic interface for the interrupt conroller mechanism.
pub trait Controller {
    /// Called once at startup to perform global initialization.
    fn global_initialize(&self);

    /// Called once per core to initialize any per-core state.
    fn initialize_for_core(&self);

    /// Interpret the contents of an `interrupts` property in a device tree node for this interrupt
    /// controller. The `index` selects which interrupt in the list to return.
    ///
    /// Returns the id and trigger mode for the interrupt.
    fn interrupt_in_device_tree(&self, data: &[u8], index: usize) -> Option<(Id, TriggerMode)>;

    /// Set the configuration of an interrupt.
    fn configure(&self, id: Id, config: &Config);

    /// Enable an interrupt to raise an exception.
    fn enable(&self, id: Id);
    /// Disable an interrupt from raising an exception.
    fn disable(&self, id: Id);

    /// Clear the pending state for this interrupt.
    fn clear_pending(&self, id: Id);

    /// Acknowledge that an interrupt exception has been handled.
    /// Returns the ID of the interrupt that was triggered.
    fn ack_interrupt(&self) -> Option<Id>;

    /// Inform the interrupt controller that the system has finished processing an interrupt.
    fn finish_interrupt(&self, id: Id);
}

/// Interrupt handler policy.
pub struct Handler<'ic, 't, T: SystemTimer> {
    controller: &'ic (dyn Controller + Sync),
    timer: &'t T,
}

/// An error that could occur during handling an interrupt.
#[derive(Debug)]
pub enum Error {
    /// An interrupt occurred that was unexpected.
    UnknownInterrupt(Id),
}

impl<'ic, 't, T: SystemTimer> Handler<'ic, 't, T> {
    /// Create a new interrupt handler policy.
    pub fn new(controller: &'ic (dyn Controller + Sync), timer: &'t T) -> Self {
        Self { controller, timer }
    }

    /// Acknowledge any interrupts that have occurred, and handle the ones that are known.
    ///
    /// # Errors
    /// - [`Error::UnknownInterrupt`]: If an interrupt happens that is unknown to the handler.
    pub fn process_interrupts(&self) -> Result<(), Error> {
        while let Some(int_id) = self.controller.ack_interrupt() {
            trace!("handling interrupt {int_id}");

            if int_id == self.timer.interrupt_id() {
                debug!("timer interrupt");
                // TODO: run scheduler here
                self.timer.reset();
            } else {
                return Err(Error::UnknownInterrupt(int_id));
            }

            trace!("finished interrupt {int_id}");
            self.controller.finish_interrupt(int_id);
        }

        Ok(())
    }
}
