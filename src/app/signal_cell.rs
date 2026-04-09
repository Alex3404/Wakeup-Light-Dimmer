use core::cell::RefCell;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;

/// Generic reactive cell that wraps a value and signals changes
pub struct SignalCell<T> {
    data: Mutex<CriticalSectionRawMutex, RefCell<T>>,
    changed: Signal<CriticalSectionRawMutex, ()>,
}

impl<T> SignalCell<T> {
    pub fn new(value: T) -> Self {
        Self {
            data: Mutex::new(RefCell::new(value)),
            changed: Signal::new(),
        }
    }

    /// Read the current value with a closure
    pub fn get<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        self.data.lock(|cell| f(&*cell.borrow()))
    }

    /// Mutate the value and signal change
    pub fn set<F>(&self, f: F)
    where
        F: FnOnce(&mut T),
    {
        self.data.lock(|cell| {
            f(&mut *cell.borrow_mut());
            self.changed.signal(());
        });
    }

    /// Get a reference to the change signal
    pub fn signal(&self) -> &Signal<CriticalSectionRawMutex, ()> {
        &self.changed
    }
}
