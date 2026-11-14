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
    /// Words queued for `read_word` (`None` = the user cancels).
    words: VecDeque<Option<String>>,
    /// Answer `read_word` from the phrase pages shown on the SCREEN (what a user
    /// copying from the device would type).
    auto_words: bool,
    shown: std::collections::BTreeMap<usize, String>,
    secrets: VecDeque<Option<String>>,
    default_secret: Option<String>,
}

/// Cloneable handle: give one clone to the `Device`, keep another to script it.
#[derive(Clone, Default)]
pub struct ScriptedHal {
    inner: Arc<Mutex<Inner>>,
}

impl ScriptedHal {
    pub fn new() -> Self {
        let h = Self::default();
        {
            let mut g = h.lock();
            g.default_decision = Some(Decision::Confirm);
            g.default_secret = Some(String::new());
        }
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
    pub fn push_word(&self, w: Option<&str>) {
        self.lock().words.push_back(w.map(String::from));
    }
    pub fn set_auto_words(&self, on: bool) {
        self.lock().auto_words = on;
    }
    pub fn push_secret(&self, s: Option<&str>) {
        self.lock().secrets.push_back(s.map(String::from));
    }
    /// The recovery phrase as shown on the device screen so far, in position order.
    pub fn shown_words(&self) -> Vec<String> {
        self.lock().shown.values().cloned().collect()
    }
    pub fn clear_shown_words(&self) {
        self.lock().shown.clear();
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
        let mut g = self.lock();
        if screen.title.starts_with("FRASE ") {
            for line in &screen.lines {
                let mut it = line.split_whitespace();
                if let (Some(n), Some(w), None) = (it.next(), it.next(), it.next())
                    && let Ok(n) = n.parse::<usize>()
                {
                    g.shown.insert(n, w.to_string());
                }
            }
        }
        g.screens.push(screen.clone());
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

    fn read_word(&mut self, position: usize) -> Option<Zeroizing<String>> {
        let mut g = self.lock();
        if let Some(w) = g.words.pop_front() {
            return w.map(Zeroizing::new);
        }
        if g.auto_words {
            return g.shown.get(&position).cloned().map(Zeroizing::new);
        }
        None
    }

    fn read_secret_text(&mut self, _prompt: &str) -> Option<Zeroizing<String>> {
        let mut g = self.lock();
        let s = match g.secrets.pop_front() {
            Some(s) => s,
            None => g.default_secret.clone(),
        };
        s.map(Zeroizing::new)
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
