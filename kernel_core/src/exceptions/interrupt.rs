//! Interrupts are exceptions caused by hardware devices.

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

/// The configuration of an interrupt with the interrupt controller.
#[derive(Debug, Default)]
pub struct Config {
    /// Priority level.
    pub priority: u8,
    /// CPU that will handle the interrupt.
    pub target_cpu: u8,
    /// Triggering mode for the interrupt.
    pub mode: TriggerMode,
}

/// An interrupt controller manages and collates interrupts for the processor.
/// This is the generic interface for the interrupt conroller mechanism.
pub trait Controller {
    /// Set the configuration of an interrupt.
    fn configure(&self, id: Id, config: &Config);

    /// Enable an interrupt to raise an exception.
    fn enable(&self, id: Id);
    /// Disable an interrupt from raising an exception.
    fn disable(&self, id: Id);

    /// Check to see if this interrupt is pending.
    fn is_pending(&self, id: Id) -> bool;
    /// Clear the pending state for this interrupt.
    fn clear_pending(&self, id: Id);

    /// Acknowledge that an interrupt exception has been handled.
    /// Returns the ID of the interrupt that was triggered.
    fn ack_interrupt(&self) -> Option<Id>;

    /// Inform the interrupt controller that the system has finished processing an interrupt.
    fn finish_interrupt(&self, id: Id);
}

/// Interrupt handler policy.
pub struct Handler<'ic, IC: Controller> {
    controller: &'ic IC,
}

/// An error that could occur during handling an interrupt.
pub enum Error {}

impl<IC: Controller> Handler<'_, IC> {
    /// Handle an interrupt that has occurred.
    pub fn handle_interrupt(&self, registers: &mut super::Registers) -> Result<(), Error> {
        while let Some(int_id) = self.controller.ack_interrupt() {
            self.controller.finish_interrupt(int_id);
        }

        Ok(())
    }
}
