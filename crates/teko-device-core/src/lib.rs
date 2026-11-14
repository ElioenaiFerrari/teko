//! Device core: state machine, PIN policy, key custody and request handling.
//!
//! Runs inside the "trusted" side. Everything sensitive (PIN entry, on-screen
//! confirmation) goes through the HAL, never through the host protocol.
#![cfg_attr(not(any(test, feature = "testing")), no_std)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

extern crate alloc;

mod config;
mod device;
mod hal;
mod pin;
mod state;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
#[cfg(test)]
mod tests;
mod text;

pub use config::DeviceConfig;
pub use device::{Device, DeviceError};
pub use hal::{Buttons, Clock, Decision, Display, Hal, HostLink, Screen, SecureStorage};
