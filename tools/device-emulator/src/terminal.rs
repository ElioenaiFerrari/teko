use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use teko_device_core::{Buttons, Clock, Decision, Display, HostLink, Screen, SecureStorage};
use teko_protocol::{ButtonKind, Response};
use zeroize::Zeroizing;

const WIDTH: usize = 34;

fn paint(color: bool, code: &str, s: &str) -> String {
    if color {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

/// Draws the device "screen" as a box (pure function: easy to test).
pub fn render_screen(screen: &Screen, color: bool) -> String {
    let pad = |s: &str| {
        let n = s.chars().count();
        format!("{s}{}", " ".repeat(WIDTH.saturating_sub(n)))
    };
    let edge = "─".repeat(WIDTH + 2);
    let mut out = String::new();
    out.push_str(&paint(color, "2", &format!("┌{edge}┐")));
    out.push('\n');
    let title = pad(&screen.title);
    out.push_str(&format!(
        "{} {} {}\n",
        paint(color, "2", "│"),
        paint(color, "1;33", &title),
        paint(color, "2", "│")
    ));
    out.push_str(&paint(color, "2", &format!("├{edge}┤")));
    out.push('\n');
    for l in &screen.lines {
        out.push_str(&format!(
            "{} {} {}\n",
            paint(color, "2", "│"),
            pad(l),
            paint(color, "2", "│")
        ));
    }
    out.push_str(&paint(color, "2", &format!("└{edge}┘")));
    out
}

fn read_line() -> Option<String> {
    let mut s = String::new();
    match io::stdin().lock().read_line(&mut s) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(s.trim().to_string()),
    }
}

/// Terminal-based screen + buttons. Keyboard = the device's physical buttons.
pub struct TerminalHal {
    color: bool,
    interactive: bool,
}

impl Default for TerminalHal {
    fn default() -> Self {
        Self::new()
    }
}

impl TerminalHal {
    pub fn new() -> Self {
        Self {
            color: io::stdout().is_terminal(),
            interactive: io::stdin().is_terminal(),
        }
    }
}

impl Display for TerminalHal {
    fn show(&mut self, screen: &Screen) {
        println!("\n{}", render_screen(screen, self.color));
        let _ = io::stdout().flush();
    }
}

impl Buttons for TerminalHal {
    fn confirm(&mut self, kind: ButtonKind) -> Decision {
        let accepted = if kind == ButtonKind::Wipe {
            println!(
                "{}",
                paint(
                    self.color,
                    "1;31",
                    "  Digite APAGAR para confirmar (Enter = cancelar): "
                )
            );
            read_line().is_some_and(|l| l == "APAGAR")
        } else {
            println!(
                "{}",
                paint(self.color, "1;32", "  [Enter] CONFIRMAR    [n] REJEITAR")
            );
            read_line().is_some_and(|l| matches!(l.to_lowercase().as_str(), "" | "y" | "s" | "sim"))
        };
        if accepted {
            Decision::Confirm
        } else {
            Decision::Reject
        }
    }

    fn read_pin(&mut self, prompt: &str) -> Option<Zeroizing<Vec<u8>>> {
        let label = format!("  {prompt} (Enter vazio = cancelar): ");
        let pin = if self.interactive {
            rpassword::prompt_password(&label).ok()
        } else {
            println!("{label}");
            read_line()
        };
        pin.filter(|p| !p.is_empty())
            .map(|p| Zeroizing::new(p.into_bytes()))
    }
}

impl HostLink for TerminalHal {
    fn notify(&mut self, interim: &Response) {
        if matches!(interim, Response::ButtonRequest(_)) {
            println!(
                "{}",
                paint(self.color, "2", "  (a Suite aguarda sua ação aqui)")
            );
        }
    }
}

/// Persists device state in a file (atomic write, owner-only permissions).
/// LIMITATION (documented in ARCHITECTURE): a file offers no rollback
/// protection for the PIN counter, unlike real secure hardware.
pub struct FileStorage {
    path: PathBuf,
}

impl FileStorage {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl SecureStorage for FileStorage {
    fn load(&mut self) -> Option<Vec<u8>> {
        fs::read(&self.path).ok()
    }

    fn store(&mut self, data: &[u8]) -> bool {
        let tmp = self.path.with_extension("tmp");
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let written = opts
            .open(&tmp)
            .and_then(|mut f| f.write_all(data).and_then(|()| f.sync_all()));
        written.is_ok() && fs::rename(&tmp, &self.path).is_ok()
    }
}

/// Wall clock in unix milliseconds (consistent across restarts, which the
/// persisted PIN back-off needs).
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or(0)
    }
}

/// Composes terminal display + file storage + system clock.
pub struct EmuHal {
    pub term: TerminalHal,
    pub storage: FileStorage,
}

impl Display for EmuHal {
    fn show(&mut self, screen: &Screen) {
        self.term.show(screen);
    }
}
impl Buttons for EmuHal {
    fn confirm(&mut self, kind: ButtonKind) -> Decision {
        self.term.confirm(kind)
    }
    fn read_pin(&mut self, prompt: &str) -> Option<Zeroizing<Vec<u8>>> {
        self.term.read_pin(prompt)
    }
}
impl SecureStorage for EmuHal {
    fn load(&mut self) -> Option<Vec<u8>> {
        self.storage.load()
    }
    fn store(&mut self, data: &[u8]) -> bool {
        self.storage.store(data)
    }
}
impl Clock for EmuHal {
    fn now_ms(&self) -> u64 {
        SystemClock.now_ms()
    }
}
impl HostLink for EmuHal {
    fn notify(&mut self, interim: &Response) {
        self.term.notify(interim);
    }
}

/// Headless HAL with AUTOMATED buttons. Only exists with the `test-buttons`
/// feature; it must never ship in a normal build.
#[cfg(feature = "test-buttons")]
pub struct AutoHal {
    pub inner: EmuHal,
    pub pin: String,
}

#[cfg(feature = "test-buttons")]
impl Display for AutoHal {
    fn show(&mut self, screen: &Screen) {
        self.inner.show(screen);
    }
}
#[cfg(feature = "test-buttons")]
impl Buttons for AutoHal {
    fn confirm(&mut self, _kind: ButtonKind) -> Decision {
        println!("  [auto] CONFIRMADO");
        Decision::Confirm
    }
    fn read_pin(&mut self, _prompt: &str) -> Option<Zeroizing<Vec<u8>>> {
        println!("  [auto] PIN digitado");
        Some(Zeroizing::new(self.pin.clone().into_bytes()))
    }
}
#[cfg(feature = "test-buttons")]
impl SecureStorage for AutoHal {
    fn load(&mut self) -> Option<Vec<u8>> {
        self.inner.load()
    }
    fn store(&mut self, data: &[u8]) -> bool {
        self.inner.store(data)
    }
}
#[cfg(feature = "test-buttons")]
impl Clock for AutoHal {
    fn now_ms(&self) -> u64 {
        SystemClock.now_ms()
    }
}
#[cfg(feature = "test-buttons")]
impl HostLink for AutoHal {
    fn notify(&mut self, interim: &Response) {
        self.inner.notify(interim);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    #[test]
    fn screen_box_has_constant_width() {
        let s = Screen::new(
            "ASSINAR?",
            vec!["linha curta".into(), "ção ã é — acentuação".into()],
        );
        let out = render_screen(&s, false);
        let widths: Vec<usize> = out.lines().map(|l| l.chars().count()).collect();
        assert!(widths.iter().all(|w| *w == WIDTH + 4), "{widths:?}");
    }

    #[test]
    fn file_storage_roundtrip_is_atomic() {
        let dir = std::env::temp_dir().join(format!("teko-emu-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let mut st = FileStorage::new(dir.join("state.bin"));
        assert!(st.load().is_none());
        assert!(st.store(b"abc"));
        assert!(st.store(b"abcdef"));
        assert_eq!(st.load().unwrap(), b"abcdef");
        assert!(!dir.join("state.tmp").exists());
        let _ = fs::remove_dir_all(dir);
    }
}
