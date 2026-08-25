pub mod alarm;
pub mod core;
pub mod ui;
pub mod app_state;

pub use crate::io::{split, AppPeripherals, RtosPeripherals};
pub use core::App;
pub use app_state::AppState;