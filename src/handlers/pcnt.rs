extern crate alloc;

use core::cell::RefCell;

use critical_section::{CriticalSection, Mutex};
use esp_hal::{
    handler,
    interrupt::Priority,
    pcnt::{Pcnt, unit},
};

type UnitInterruptFunction<const UNIT: usize> =
    fn(cs: CriticalSection<'_>, &unit::Unit<'_, UNIT>) -> ();

struct UserInterrupt<const UNIT: usize> {
    unit: unit::Unit<'static, UNIT>,
    handler: UnitInterruptFunction<UNIT>,
}

// Defines a anonomus function
macro_rules! interrupt_references {
    ($($number:literal),*) => {
        $(
            paste::paste!{
                static [<UNIT_ $number _INTERRUPT_HANDLER>]
                    : Mutex<RefCell<Option<UserInterrupt<$number>>>>
                    = Mutex::new(RefCell::new(None));
            }
        )+
    };
}

macro_rules! add_interrupt_functions {
    ($($number:literal),*) => {
        $(
            paste::paste! {
                pub fn [<add_handler_to_unit $number>](handler: UnitInterruptFunction<$number>, unit: unit::Unit<'static, $number>) {
                    critical_section::with(|cs| {
                        let mut handler_cell = [<UNIT_ $number _INTERRUPT_HANDLER>].borrow_ref_mut(cs);
                        if handler_cell.is_some() {
                            panic!(concat!("Duplicate pcnt user interrupt handler for unit ", $number));
                        }
                        handler_cell.replace(UserInterrupt { unit, handler })
                    });
                }
            }
        )+
    };
}

macro_rules! unit_interupt_handlers {
    ($cs:expr, $($number:literal),*) => {
        $(
            paste::paste! {{
                let cell = [<UNIT_ $number _INTERRUPT_HANDLER>].borrow_ref($cs);
                if let Some(ref unit_int_handler) = *cell {
                    if unit_int_handler.unit.interrupt_is_set() {
                        (unit_int_handler.handler)($cs, &unit_int_handler.unit);
                        unit_int_handler.unit.reset_interrupt();
                    }
                }
            }}
        )+
    };
}

#[cfg(esp32)]
interrupt_references!(0, 1, 2, 3, 4, 5, 6, 7);
#[cfg(not(esp32))]
interrupt_references!(0, 1, 2, 3);

#[cfg(esp32)]
add_interrupt_functions!(0, 1, 2, 3, 4, 5, 6, 7);
#[cfg(not(esp32))]
add_interrupt_functions!(0, 1, 2, 3);

#[handler(priority = Priority::Priority3)]
fn interrupt_handler() {
    critical_section::with(|cs| {
        #[cfg(esp32)]
        unit_interupt_handlers!(cs, 0, 1, 2, 3, 4, 5, 6, 7);
        #[cfg(not(esp32))]
        unit_interupt_handlers!(cs, 0, 1, 2, 3);
    });
}

pub fn initalize(pcnt: &mut Pcnt<'_>) {
    pcnt.set_interrupt_handler(interrupt_handler);
}
