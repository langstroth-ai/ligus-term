//! A test harness: the real reducer and the fake backend, with effects run
//! as tasks (some wait on a passkey answer) and their results fed back.

#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use futures::StreamExt;
use futures::stream::BoxStream;
use ligus_term::app::runtime::run_effect;
use ligus_term::app::{Effect, Msg, State, update};
use ligus_term::backend::fake::{DEMO_NOW, FakeBackend, FakeConfig};
use ligus_term::backend::{Backend, Event};
use ligus_term::keymap::Keymap;
use ligus_term::theme::Theme;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tokio::sync::mpsc;

pub struct Harness {
    pub backend: Arc<dyn Backend>,
    pub state: State,
    events: BoxStream<'static, Event>,
    tx: mpsc::UnboundedSender<Msg>,
    rx: mpsc::UnboundedReceiver<Msg>,
}

impl Harness {
    pub async fn new(demo: bool) -> Harness {
        let cfg = FakeConfig {
            stream_delay: Duration::ZERO,
            browser_delay: Duration::from_millis(5),
            data_dir: "~/.ligus-term".into(),
            ..FakeConfig::default()
        };
        let fake = if demo { FakeBackend::demo(cfg) } else { FakeBackend::new(cfg) };
        Self::with(fake).await
    }

    /// The fake backend's settings for tests.
    pub fn cfg() -> FakeConfig {
        FakeConfig {
            stream_delay: Duration::ZERO,
            browser_delay: Duration::from_millis(5),
            data_dir: "~/.ligus-term".into(),
            ..FakeConfig::default()
        }
    }

    /// A harness over a given fake backend.
    pub async fn with(fake: FakeBackend) -> Harness {
        let backend: Arc<dyn Backend> = Arc::new(fake);
        let mut state = State::new(backend.device(), Keymap::default(), DEMO_NOW);
        state.config_path = Some("~/.config/ligus-term/config.toml".into());
        let events = backend.events();
        let (tx, rx) = mpsc::unbounded_channel();
        let mut h = Harness { backend, state, events, tx, rx };
        h.send(Msg::Startup(h.backend.startup().await)).await;
        h
    }

    /// Apply a message and run what it asks for until things settle.
    pub async fn send(&mut self, msg: Msg) {
        self.apply(msg);
        self.settle().await;
    }

    fn apply(&mut self, msg: Msg) {
        for fx in update(&mut self.state, msg) {
            // The diff preview's wait is the runtime's; here the selection
            // has rested as soon as the reducer asks.
            if let Effect::PreviewDiff(w, p) = fx {
                let _ = self.tx.send(Msg::PreviewDue(w, p));
                continue;
            }
            let (b, tx) = (self.backend.clone(), self.tx.clone());
            tokio::spawn(async move {
                let _ = tx.send(run_effect(b, fx).await);
            });
        }
    }

    /// Feed effect results and backend events until 60 ms pass quietly.
    pub async fn settle(&mut self) {
        loop {
            tokio::select! {
                Some(m) = self.rx.recv() => self.apply(m),
                Some(e) = self.events.next() => self.apply(Msg::Event(e)),
                _ = tokio::time::sleep(Duration::from_millis(60)) => break,
            }
        }
    }

    pub async fn key(&mut self, code: KeyCode) {
        self.send(Msg::Key(KeyEvent::new(code, KeyModifiers::NONE))).await;
    }

    pub async fn ctrl(&mut self, c: char) {
        self.send(Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))).await;
    }

    pub async fn chars(&mut self, s: &str) {
        for c in s.chars() {
            self.key(KeyCode::Char(c)).await;
        }
    }

    /// Render to text at `w`×`h` with the dark theme.
    pub fn screen(&self, w: u16, h: u16) -> String {
        render(&self.state, w, h, &Theme::DARK)
    }
}

pub fn render(state: &State, w: u16, h: u16, theme: &Theme) -> String {
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| ligus_term::ui::draw(f, state, theme)).unwrap();
    buffer_text(term.backend().buffer())
}

pub fn buffer_text(buf: &ratatui::buffer::Buffer) -> String {
    let area = buf.area;
    let mut out = String::new();
    for y in 0..area.height {
        let mut line = String::new();
        let mut skip = 0;
        for x in 0..area.width {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            let sym = buf[(x, y)].symbol();
            line.push_str(sym);
            skip = unicode_width::UnicodeWidthStr::width(sym).saturating_sub(1);
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}
