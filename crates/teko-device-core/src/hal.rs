use alloc::string::String;
use alloc::vec::Vec;

use teko_protocol::{ButtonKind, Response};
use zeroize::Zeroizing;

/// What the device screen shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screen {
    pub title: String,
    pub lines: Vec<String>,
}

impl Screen {
    pub fn new(title: &str, lines: Vec<String>) -> Self {
        Self {
            title: String::from(title),
            lines,
        }
    }
}

/// Physical user decision on the device buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Confirm,
    Reject,
    Timeout,
}

pub trait Display {
    fn show(&mut self, screen: &Screen);
}

pub trait Buttons {
    /// Blocks until the user confirms/rejects ON THE DEVICE.
    /// `kind` lets the device UI demand a stronger gesture (e.g. for wipe).
    fn confirm(&mut self, kind: ButtonKind) -> Decision;
    /// Reads a PIN typed ON THE DEVICE. `None` = user cancelled.
    fn read_pin(&mut self, prompt: &str) -> Option<Zeroizing<Vec<u8>>>;
}

pub trait SecureStorage {
    fn load(&mut self) -> Option<Vec<u8>>;
    /// Must persist atomically; returns false on failure.
    fn store(&mut self, data: &[u8]) -> bool;
}

pub trait Clock {
    fn now_ms(&self) -> u64;
}

/// Pushes interim messages (e.g. `ButtonRequest`) to the host mid-request.
pub trait HostLink {
    fn notify(&mut self, interim: &Response);
}

pub trait Hal: Display + Buttons + SecureStorage + Clock + HostLink {}
impl<T: Display + Buttons + SecureStorage + Clock + HostLink> Hal for T {}
