//! Device emulator: the "screen and buttons" live HERE, in a separate process
//! from the Suite. PIN entry and every confirmation happen in this terminal.

mod link;
mod server;
mod terminal;

pub use link::{LinkSlot, Linked};
pub use server::{IDLE_TIMEOUT, handle_connection, handle_connection_with, serve};
#[cfg(feature = "test-buttons")]
pub use terminal::AutoHal;
pub use terminal::{EmuHal, FileStorage, SystemClock, TerminalHal, render_screen};
