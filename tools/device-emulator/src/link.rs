use std::io::Write;
use std::net::TcpStream;
use std::sync::{Arc, Mutex};

use teko_device_core::{Buttons, Clock, Decision, Display, HostLink, Screen, SecureStorage};
use teko_protocol::{ButtonKind, Response};
use zeroize::Zeroizing;

/// The connection (and request id) interim messages are pushed to.
#[derive(Clone, Default)]
pub struct LinkSlot(Arc<Mutex<Option<(TcpStream, u32)>>>);

impl LinkSlot {
    pub fn set(&self, stream: TcpStream, request_id: u32) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some((stream, request_id));
    }

    pub fn clear(&self) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// Wraps any HAL and adds the host link (so `ButtonRequest` reaches the Suite).
pub struct Linked<H> {
    pub inner: H,
    slot: LinkSlot,
}

impl<H> Linked<H> {
    pub fn new(inner: H, slot: LinkSlot) -> Self {
        Self { inner, slot }
    }
}

impl<H: Display> Display for Linked<H> {
    fn show(&mut self, screen: &Screen) {
        self.inner.show(screen);
    }
}

impl<H: Buttons> Buttons for Linked<H> {
    fn confirm(&mut self, kind: ButtonKind) -> Decision {
        self.inner.confirm(kind)
    }

    fn read_pin(&mut self, prompt: &str) -> Option<Zeroizing<Vec<u8>>> {
        self.inner.read_pin(prompt)
    }

    fn read_word(&mut self, position: usize) -> Option<Zeroizing<String>> {
        self.inner.read_word(position)
    }

    fn read_secret_text(&mut self, prompt: &str) -> Option<Zeroizing<String>> {
        self.inner.read_secret_text(prompt)
    }
}

impl<H: SecureStorage> SecureStorage for Linked<H> {
    fn load(&mut self) -> Option<Vec<u8>> {
        self.inner.load()
    }

    fn store(&mut self, data: &[u8]) -> bool {
        self.inner.store(data)
    }
}

impl<H: Clock> Clock for Linked<H> {
    fn now_ms(&self) -> u64 {
        self.inner.now_ms()
    }
}

impl<H: HostLink> HostLink for Linked<H> {
    fn notify(&mut self, interim: &Response) {
        if let Some((stream, id)) = self
            .slot
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            let _ = (&*stream).write_all(&interim.to_frame(*id).encode());
        }
        self.inner.notify(interim);
    }
}
