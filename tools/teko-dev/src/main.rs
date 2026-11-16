//! The terminal side of `teko-dev` (see `lib.rs`): a full-screen menu over the make targets.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, ExitCode, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use teko_dev::{CATALOG, Item, Mode, NetState, command_lines, probe, validate};

const GREEN: Color = Color::Rgb(0, 155, 58);
const GOLD: Color = Color::Rgb(254, 223, 0);
const DIM: Color = Color::DarkGray;

/// One row of the menu: a group title or an item.
enum Row {
    Group(&'static str),
    Item(&'static Item),
}

fn rows() -> Vec<Row> {
    let mut out = Vec::new();
    let mut last = "";
    for i in CATALOG {
        if i.group != last {
            out.push(Row::Group(i.group));
            last = i.group;
        }
        out.push(Row::Item(i));
    }
    out
}

enum Msg {
    Line(String),
    Done(Option<i32>),
}

struct Running {
    title: String,
    lines: Vec<String>,
    rx: Receiver<Msg>,
    child: Arc<Mutex<Option<Child>>>,
    done: Option<Option<i32>>,
    /// Lines scrolled up from the bottom (0 = follow the output).
    back: usize,
}

struct Form {
    item: &'static Item,
    values: Vec<String>,
    focus: usize,
    error: String,
}

enum Screen {
    Menu,
    Form(Form),
    Running(Running),
}

struct App {
    rows: Vec<Row>,
    sel: usize,
    list: ListState,
    net: NetState,
    net_rx: Receiver<NetState>,
    screen: Screen,
    message: String,
    dir: PathBuf,
}

impl App {
    fn new(dir: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel();
        let d = dir.clone();
        std::thread::spawn(move || {
            loop {
                if tx.send(probe(&d)).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        });
        let rows = rows();
        let sel = rows
            .iter()
            .position(|r| matches!(r, Row::Item(_)))
            .unwrap_or(0);
        let mut list = ListState::default();
        list.select(Some(sel));
        Self {
            rows,
            sel,
            list,
            net: NetState::Unknown,
            net_rx: rx,
            screen: Screen::Menu,
            message: String::new(),
            dir,
        }
    }

    fn selected_item(&self) -> Option<&'static Item> {
        match self.rows.get(self.sel) {
            Some(Row::Item(i)) => Some(i),
            _ => None,
        }
    }

    fn step(&mut self, down: bool) {
        let n = self.rows.len();
        let mut i = self.sel;
        for _ in 0..n {
            i = if down { (i + 1) % n } else { (i + n - 1) % n };
            if matches!(self.rows.get(i), Some(Row::Item(_))) {
                break;
            }
        }
        self.sel = i;
        self.list.select(Some(i));
        self.message.clear();
    }
}

fn main() -> ExitCode {
    // Ctrl-C while a command owns the terminal must end THAT command, not this menu: a handler (not
    // SIG_IGN) is reset to the default in children, so they still die on it.
    #[cfg(unix)]
    {
        extern "C" fn noop(_: libc::c_int) {}
        // SAFETY: installing a handler that does nothing is async-signal-safe.
        unsafe {
            libc::signal(
                libc::SIGINT,
                noop as extern "C" fn(libc::c_int) as libc::sighandler_t,
            );
        }
    }
    let dir = PathBuf::from(std::env::var("NET").unwrap_or_else(|_| "dev/net".into()));
    if !std::path::Path::new("Makefile").exists() {
        eprintln!("erro: rode `make dev` na raiz do repositório (não achei o Makefile)");
        return ExitCode::FAILURE;
    }
    let mut terminal = ratatui::init();
    let result = run(&mut terminal, App::new(dir));
    ratatui::restore();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("erro: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(terminal: &mut DefaultTerminal, mut app: App) -> Result<(), String> {
    loop {
        while let Ok(n) = app.net_rx.try_recv() {
            app.net = n;
        }
        if let Screen::Running(r) = &mut app.screen {
            pump(r);
        }
        terminal
            .draw(|f| draw(f, &mut app))
            .map_err(|e| e.to_string())?;
        if !event::poll(Duration::from_millis(100)).map_err(|e| e.to_string())? {
            continue;
        }
        let Event::Key(k) = event::read().map_err(|e| e.to_string())? else {
            continue;
        };
        if k.kind != KeyEventKind::Press {
            continue;
        }
        if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
            if let Screen::Running(r) = &mut app.screen {
                kill(r);
                continue;
            }
            return Ok(());
        }
        match &mut app.screen {
            Screen::Menu => {
                if on_menu_key(&mut app, k, terminal)? {
                    return Ok(());
                }
            }
            Screen::Form(_) => on_form_key(&mut app, k, terminal)?,
            Screen::Running(_) => on_running_key(&mut app, k),
        }
    }
}

// ───────────────────────── input ─────────────────────────

/// Returns true to quit.
fn on_menu_key(app: &mut App, k: KeyEvent, terminal: &mut DefaultTerminal) -> Result<bool, String> {
    match k.code {
        KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
        KeyCode::Down | KeyCode::Char('j') => app.step(true),
        KeyCode::Up | KeyCode::Char('k') => app.step(false),
        KeyCode::Enter => {
            let Some(item) = app.selected_item() else {
                return Ok(false);
            };
            if let Err(why) = item.available(&app.net) {
                app.message = format!("indisponível agora: {why}");
            } else if item.params.is_empty() {
                launch(app, item, Vec::new(), terminal)?;
            } else {
                app.screen = Screen::Form(Form {
                    item,
                    values: item.params.iter().map(|p| p.default.to_string()).collect(),
                    focus: 0,
                    error: String::new(),
                });
            }
        }
        _ => {}
    }
    Ok(false)
}

fn on_form_key(app: &mut App, k: KeyEvent, terminal: &mut DefaultTerminal) -> Result<(), String> {
    let Screen::Form(f) = &mut app.screen else {
        return Ok(());
    };
    let n = f.item.params.len();
    match k.code {
        KeyCode::Esc => app.screen = Screen::Menu,
        KeyCode::Tab | KeyCode::Down => f.focus = (f.focus + 1) % n,
        KeyCode::BackTab | KeyCode::Up => f.focus = (f.focus + n - 1) % n,
        KeyCode::Left | KeyCode::Right => {
            let p = &f.item.params[f.focus];
            if !p.choices.is_empty() {
                let cur = p
                    .choices
                    .iter()
                    .position(|c| *c == f.values[f.focus])
                    .unwrap_or(0);
                let next = if k.code == KeyCode::Right {
                    (cur + 1) % p.choices.len()
                } else {
                    (cur + p.choices.len() - 1) % p.choices.len()
                };
                f.values[f.focus] = p.choices[next].to_string();
            }
        }
        KeyCode::Backspace => {
            if f.item.params[f.focus].choices.is_empty() {
                f.values[f.focus].pop();
            }
        }
        KeyCode::Char(c) => {
            if f.item.params[f.focus].choices.is_empty() && f.values[f.focus].len() < 80 {
                f.values[f.focus].push(c);
            }
        }
        KeyCode::Enter => {
            let (item, values) = (f.item, f.values.clone());
            let bad = item
                .params
                .iter()
                .zip(&values)
                .enumerate()
                .find_map(|(i, (p, v))| validate(p, v).err().map(|e| (i, e)));
            if let Some((i, e)) = bad {
                f.focus = i;
                f.error = e;
            } else {
                app.screen = Screen::Menu;
                launch(app, item, values, terminal)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn on_running_key(app: &mut App, k: KeyEvent) {
    let Screen::Running(r) = &mut app.screen else {
        return;
    };
    match k.code {
        KeyCode::Up => r.back = r.back.saturating_add(1),
        KeyCode::Down => r.back = r.back.saturating_sub(1),
        KeyCode::PageUp => r.back = r.back.saturating_add(15),
        KeyCode::PageDown => r.back = r.back.saturating_sub(15),
        KeyCode::End => r.back = 0,
        KeyCode::Char('k') if r.done.is_none() => kill(r),
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter if r.done.is_some() => {
            app.screen = Screen::Menu;
        }
        _ => {}
    }
}

// ───────────────────────── running commands ─────────────────────────

fn launch(
    app: &mut App,
    item: &'static Item,
    values: Vec<String>,
    terminal: &mut DefaultTerminal,
) -> Result<(), String> {
    let lines = match command_lines(item, &values) {
        Ok(l) => l,
        Err(e) => {
            app.message = e;
            return Ok(());
        }
    };
    let title = lines
        .iter()
        .map(|c| c.join(" "))
        .collect::<Vec<_>>()
        .join("  &&  ");
    match item.mode {
        Mode::Capture => app.screen = Screen::Running(spawn_captured(title, lines)),
        Mode::Terminal => {
            ratatui::restore();
            println!("\n$ {title}\n");
            let mut code = None;
            for c in &lines {
                match Command::new(&c[0]).args(&c[1..]).status() {
                    Ok(s) => code = s.code(),
                    Err(e) => {
                        println!("erro ao executar: {e}");
                        code = Some(127);
                    }
                }
                if code != Some(0) {
                    break;
                }
            }
            print!(
                "\n[terminou com código {}] Enter para voltar ao menu… ",
                code.map_or("?".to_string(), |c| c.to_string())
            );
            let _ = std::io::stdout().flush();
            let mut s = String::new();
            let _ = std::io::stdin().read_line(&mut s);
            *terminal = ratatui::init();
            app.net = NetState::Unknown;
        }
    }
    let _ = &app.dir;
    Ok(())
}

fn spawn_captured(title: String, lines: Vec<Vec<String>>) -> Running {
    let (tx, rx) = mpsc::channel();
    let child: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
    let slot = child.clone();
    std::thread::spawn(move || worker(&lines, &tx, &slot));
    Running {
        title,
        lines: Vec::new(),
        rx,
        child,
        done: None,
        back: 0,
    }
}

fn reader(r: impl Read + Send + 'static, tx: Sender<Msg>) {
    std::thread::spawn(move || {
        for l in BufReader::new(r).lines().map_while(Result::ok) {
            if tx.send(Msg::Line(l.replace('\t', "    "))).is_err() {
                break;
            }
        }
    });
}

/// Runs the steps in order, streaming their output. Stops at the first failure.
fn worker(steps: &[Vec<String>], tx: &Sender<Msg>, slot: &Arc<Mutex<Option<Child>>>) {
    let mut code = Some(0);
    for c in steps {
        let _ = tx.send(Msg::Line(format!("$ {}", c.join(" "))));
        let spawned = Command::new(&c[0])
            .args(&c[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let mut child = match spawned {
            Ok(ch) => ch,
            Err(e) => {
                let _ = tx.send(Msg::Line(format!("erro ao executar: {e}")));
                code = Some(127);
                break;
            }
        };
        if let Some(o) = child.stdout.take() {
            reader(o, tx.clone());
        }
        if let Some(e) = child.stderr.take() {
            reader(e, tx.clone());
        }
        if let Ok(mut g) = slot.lock() {
            *g = Some(child);
        }
        // poll without holding the lock, so `kill` can take the child
        code = loop {
            let status = slot
                .lock()
                .ok()
                .and_then(|mut g| g.as_mut().map(Child::try_wait));
            match status {
                Some(Ok(Some(s))) => break s.code(),
                Some(Ok(None)) => std::thread::sleep(Duration::from_millis(50)),
                _ => break None,
            }
        };
        // background daemons (make devnet) keep no pipe open, but give the readers a moment
        std::thread::sleep(Duration::from_millis(250));
        if code != Some(0) {
            break;
        }
    }
    let _ = tx.send(Msg::Done(code));
}

fn pump(r: &mut Running) {
    let start = Instant::now();
    while let Ok(m) = r.rx.try_recv() {
        match m {
            Msg::Line(l) => {
                r.lines.push(l);
                if r.lines.len() > 20_000 {
                    r.lines.drain(..5_000);
                }
            }
            Msg::Done(c) => r.done = Some(c),
        }
        if start.elapsed() > Duration::from_millis(40) {
            break;
        }
    }
}

fn kill(r: &mut Running) {
    if let Ok(mut g) = r.child.lock()
        && let Some(c) = g.as_mut()
    {
        let _ = c.kill();
    }
}

// ───────────────────────── drawing ─────────────────────────

fn draw(f: &mut ratatui::Frame, app: &mut App) {
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(5),
        Constraint::Length(3),
    ])
    .split(f.area());
    let (color, _) = match &app.net {
        NetState::Up { .. } => (GREEN, ()),
        NetState::Down { .. } => (GOLD, ()),
        _ => (DIM, ()),
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            app.net.headline(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Teko · desenvolvimento "),
        ),
        chunks[0],
    );
    let mut footer = "↑↓ navegar · Enter executar · q sair".to_string();
    match &mut app.screen {
        Screen::Menu => {
            draw_menu(f, chunks[1], app);
        }
        Screen::Form(form) => {
            draw_form(f, chunks[1], form);
            footer = "Tab/↑↓ campo · ←/→ opções · Enter executa · Esc cancela".into();
        }
        Screen::Running(r) => {
            draw_running(f, chunks[1], r);
            footer = if r.done.is_some() {
                "↑↓/PgUp/PgDn rolar · Enter ou Esc volta ao menu".into()
            } else {
                "rodando… · ↑↓/PgUp/PgDn rolar · k interrompe".into()
            };
        }
    }
    let msg = if app.message.is_empty() {
        footer
    } else {
        app.message.clone()
    };
    f.render_widget(
        Paragraph::new(msg)
            .style(Style::default().fg(DIM))
            .wrap(Wrap { trim: true })
            .block(Block::default().borders(Borders::ALL)),
        chunks[2],
    );
}

fn draw_menu(f: &mut ratatui::Frame, area: ratatui::layout::Rect, app: &mut App) {
    let net = app.net.clone();
    let items: Vec<ListItem> = app
        .rows
        .iter()
        .map(|r| match r {
            Row::Group(g) => ListItem::new(Line::from(Span::styled(
                format!(" {g}"),
                Style::default().fg(GOLD).add_modifier(Modifier::BOLD),
            ))),
            Row::Item(i) => {
                let ok = i.available(&net).is_ok();
                let style = if ok {
                    Style::default()
                } else {
                    Style::default().fg(DIM)
                };
                let mut spans = vec![Span::styled(format!("   {}", i.label), style)];
                if !i.params.is_empty() {
                    spans.push(Span::styled(" …", Style::default().fg(DIM)));
                }
                ListItem::new(Line::from(spans))
            }
        })
        .collect();
    let parts =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).split(area);
    f.render_stateful_widget(
        List::new(items)
            .block(Block::default().borders(Borders::ALL).title(" Comandos "))
            .highlight_style(
                Style::default()
                    .bg(GREEN)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        parts[0],
        &mut app.list,
    );
    let mut text = Vec::new();
    if let Some(i) = app.selected_item() {
        let cmd = command_lines(
            i,
            &i.params
                .iter()
                .map(|p| p.default.to_string())
                .collect::<Vec<_>>(),
        )
        .map(|l| l.iter().map(|c| c.join(" ")).collect::<Vec<_>>().join("\n"))
        .unwrap_or_else(|_| format!("make {}  (preencha os campos)", i.steps.join(" + make ")));
        text.push(Line::from(Span::styled(
            i.label,
            Style::default().add_modifier(Modifier::BOLD),
        )));
        text.push(Line::from(""));
        text.push(Line::from(Span::styled(cmd, Style::default().fg(GREEN))));
        if !i.note.is_empty() {
            text.push(Line::from(""));
            text.push(Line::from(i.note));
        }
        if let Err(why) = i.available(&net) {
            text.push(Line::from(""));
            text.push(Line::from(Span::styled(
                format!("Agora não: {why}"),
                Style::default().fg(GOLD),
            )));
        }
        if i.mode == Mode::Terminal {
            text.push(Line::from(""));
            text.push(Line::from(Span::styled(
                "Abre no terminal inteiro; ao terminar (ou com Ctrl-C) volta aqui.",
                Style::default().fg(DIM),
            )));
        }
    }
    f.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL).title(" O que roda ")),
        parts[1],
    );
}

fn draw_form(f: &mut ratatui::Frame, area: ratatui::layout::Rect, form: &Form) {
    let mut lines = vec![
        Line::from(Span::styled(
            form.item.label,
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];
    for (i, (p, v)) in form.item.params.iter().zip(&form.values).enumerate() {
        let focus = i == form.focus;
        let mark = if focus { "▶ " } else { "  " };
        let cursor = if focus && p.choices.is_empty() {
            "▏"
        } else {
            ""
        };
        let req = if p.required { " *" } else { "" };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{mark}{}{req}: ", p.label),
                Style::default().fg(if focus { GOLD } else { DIM }),
            ),
            Span::styled(
                format!("{v}{cursor}"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]));
    }
    lines.push(Line::from(""));
    let preview = command_lines(form.item, &form.values)
        .map(|l| {
            l.iter()
                .map(|c| c.join(" "))
                .collect::<Vec<_>>()
                .join("  &&  ")
        })
        .unwrap_or_default();
    lines.push(Line::from(Span::styled(
        format!("$ {preview}"),
        Style::default().fg(GREEN),
    )));
    if !form.error.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            form.error.clone(),
            Style::default().fg(Color::Red),
        )));
    }
    f.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Parâmetros (* obrigatório) "),
        ),
        area,
    );
}

fn draw_running(f: &mut ratatui::Frame, area: ratatui::layout::Rect, r: &mut Running) {
    let inner = area.height.saturating_sub(2) as usize;
    let total = r.lines.len();
    r.back = r.back.min(total.saturating_sub(inner));
    let end = total.saturating_sub(r.back);
    let start = end.saturating_sub(inner);
    let text: Vec<Line> = r.lines[start..end]
        .iter()
        .map(|l| Line::from(l.as_str()))
        .collect();
    let status = match r.done {
        None => " rodando… ".to_string(),
        Some(Some(0)) => " ✔ terminou ".to_string(),
        Some(Some(c)) => format!(" ✘ terminou com código {c} "),
        Some(None) => " ✘ interrompido ".to_string(),
    };
    f.render_widget(
        Paragraph::new(text).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {} —{status}", r.title)),
        ),
        area,
    );
}
