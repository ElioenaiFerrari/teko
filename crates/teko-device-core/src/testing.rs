//! Scripted HAL: automated "buttons" for tests and headless e2e.
//! Compiled only with `cfg(test)` or the `testing` feature — never in a normal build.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

use teko_protocol::{ButtonKind, Response};
use zeroize::Zeroizing;

use crate::hal::{Buttons, Clock, Decision, Display, HostLink, Screen, SecureStorage};

#[derive(Default)]
struct Inner {
    storage: Option<Vec<u8>>,
    now_ms: u64,
    screens: Vec<Screen>,
    interim: Vec<Response>,
    decisions: VecDeque<Decision>,
    default_decision: Option<Decision>,
    pins: VecDeque<Option<String>>,
    default_pin: Option<String>,
    confirms: usize,
}

/// Cloneable handle: give one clone to the `Device`, keep another to script it.
#[derive(Clone, Default)]
pub struct ScriptedHal {
    inner: Arc<Mutex<Inner>>,
}

impl ScriptedHal {
    pub fn new() -> Self {
        let h = Self::default();
        h.lock().default_decision = Some(Decision::Confirm);
        h
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_default_decision(&self, d: Decision) {
        self.lock().default_decision = Some(d);
    }
    pub fn push_decision(&self, d: Decision) {
        self.lock().decisions.push_back(d);
    }
    pub fn set_default_pin(&self, pin: Option<&str>) {
        self.lock().default_pin = pin.map(String::from);
    }
    pub fn push_pin(&self, pin: Option<&str>) {
        self.lock().pins.push_back(pin.map(String::from));
    }
    pub fn advance_ms(&self, ms: u64) {
        let mut g = self.lock();
        g.now_ms = g.now_ms.saturating_add(ms);
    }
    pub fn screens(&self) -> Vec<Screen> {
        self.lock().screens.clone()
    }
    pub fn clear_screens(&self) {
        self.lock().screens.clear();
    }
    pub fn interim(&self) -> Vec<Response> {
        self.lock().interim.clone()
    }
    pub fn clear_interim(&self) {
        self.lock().interim.clear();
    }
    pub fn confirms_requested(&self) -> usize {
        self.lock().confirms
    }
    pub fn raw_storage(&self) -> Option<Vec<u8>> {
        self.lock().storage.clone()
    }
}

impl Display for ScriptedHal {
    fn show(&mut self, screen: &Screen) {
        self.lock().screens.push(screen.clone());
    }
}

impl Buttons for ScriptedHal {
    fn confirm(&mut self, _kind: ButtonKind) -> Decision {
        let mut g = self.lock();
        g.confirms += 1;
        if let Some(d) = g.decisions.pop_front() {
            return d;
        }
        g.default_decision.unwrap_or(Decision::Reject)
    }

    fn read_pin(&mut self, _prompt: &str) -> Option<Zeroizing<Vec<u8>>> {
        let mut g = self.lock();
        let pin = match g.pins.pop_front() {
            Some(p) => p,
            None => g.default_pin.clone(),
        };
        pin.map(|p| Zeroizing::new(p.into_bytes()))
    }
}

impl SecureStorage for ScriptedHal {
    fn load(&mut self) -> Option<Vec<u8>> {
        self.lock().storage.clone()
    }
    fn store(&mut self, data: &[u8]) -> bool {
        self.lock().storage = Some(data.to_vec());
        true
    }
}

impl Clock for ScriptedHal {
    fn now_ms(&self) -> u64 {
        self.lock().now_ms
    }
}

impl HostLink for ScriptedHal {
    fn notify(&mut self, interim: &Response) {
        self.lock().interim.push(interim.clone());
    }
}
