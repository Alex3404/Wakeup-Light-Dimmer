use core::cell::RefCell;

use critical_section::{CriticalSection, Mutex};
use esp_hal::{
    gpio::{Event, Input, Io},
    handler,
    interrupt::Priority,
};
use heapless::Vec;
extern crate alloc;
use alloc::sync::Arc;

pub type InputHandlerReference = Arc<Mutex<RefCell<InputHandler>>>;
pub type UserInputInterruptFunction = fn(cs: CriticalSection<'_>, InputHandlerReference) -> ();

#[derive(Debug)]
pub struct InputHandler {
    input: Input<'static>,
    event: Event,
    handler: UserInputInterruptFunction,
}

impl InputHandler {
    pub fn get_input(&self) -> &Input<'static> {
        &self.input
    }

    pub fn get_input_mut(&mut self) -> &mut Input<'static> {
        &mut self.input
    }

    pub fn get_event(&self) -> Event {
        self.event
    }

    pub fn update_event(&mut self, new_event: Event) {
        self.input.listen(new_event);
        self.event = new_event;
    }
}

static INPUT_HANDLERS: Mutex<RefCell<Vec<InputHandlerReference, 32>>> =
    Mutex::new(RefCell::new(Vec::new()));

#[handler(priority = Priority::Priority3)]
fn interrupt_handler() {
    critical_section::with(|cs| {
        let handlers = INPUT_HANDLERS.borrow_ref(cs);

        for handler_reference in handlers.iter() {
            let mut borrowed_ref = handler_reference.borrow_ref_mut(cs);
            if !borrowed_ref.input.is_interrupt_set() {
                // Reference dropped
                continue;
            }
            // Clear interrupt bit on input
            borrowed_ref.input.clear_interrupt();

            // Clone the handler function
            let handler = &borrowed_ref.handler.clone();
            drop(borrowed_ref); // Drop so handler can modify the handler reference if needed

            handler(cs, handler_reference.clone());
        }
    });
}

pub fn initalize(io: &mut Io<'_>) {
    io.set_interrupt_handler(interrupt_handler);
    io.set_interrupt_priority(Priority::Priority3);
}

// Try to start listening if input handlers is full
// return the input
pub fn start_listening(
    mut input: Input<'static>,
    event: Event,
    handler: UserInputInterruptFunction,
) -> Result<InputHandlerReference, Input<'static>> {
    critical_section::with(|cs| {
        input.listen(event);
        let handler = InputHandler {
            input,
            event,
            handler,
        };

        let reference = Arc::new(Mutex::new(RefCell::new(handler)));
        let push_result = INPUT_HANDLERS.borrow_ref_mut(cs).push(reference.clone());

        if let Err(vec_reference) = push_result {
            // Failed to add vector must be full
            drop(vec_reference); // Drop only other reference

            // Deconstruct arc should work since only 1 reference exists
            let mutex = Arc::try_unwrap(reference).unwrap();
            let input = mutex.into_inner().into_inner().input;
            Err(input)
        } else {
            Ok(reference)
        }
    })
}
