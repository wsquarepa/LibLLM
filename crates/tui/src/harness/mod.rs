//! In-process verification harness for the TUI. Compiled only under the
//! `test-support` feature. Boots `App` against a `ratatui` `TestBackend`,
//! drives it with synthetic events, and exposes screen + state for assertions.

mod builder;
mod observe;

pub use builder::HarnessBuilder;
pub use observe::Observation;

use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tokio::sync::mpsc;

use crate::commands;
use crate::types::{App, BackgroundEvent};
use libllm_protocol::client::StreamToken;
use libllm_protocol::tokenizer::TokenCountUpdate;

/// Drives a fully-constructed `App` against a `TestBackend`. The caller owns the
/// `Session` (mirroring `run()`); the harness borrows it for `'a`. Each driving
/// call renders a frame, matching the real loop which draws after every event.
pub struct Harness<'a> {
    pub(crate) app: App<'a>,
    terminal: Terminal<TestBackend>,
    token_tx: mpsc::Sender<StreamToken>,
    token_rx: mpsc::Receiver<StreamToken>,
    bg_tx: mpsc::Sender<BackgroundEvent>,
    bg_rx: mpsc::Receiver<BackgroundEvent>,
    tokenizer_rx: mpsc::Receiver<TokenCountUpdate>,
    /// Keeps the temp DB directory alive for the harness lifetime.
    _tempdir: Option<tempfile::TempDir>,
}

impl<'a> Harness<'a> {
    pub fn builder() -> HarnessBuilder {
        HarnessBuilder::new()
    }

    fn render(&mut self) {
        let app = &mut self.app;
        self.terminal
            .draw(|f| crate::render_frame(f, app))
            .expect("test backend draw");
    }

    pub async fn key(&mut self, code: KeyCode) {
        self.send_event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
            .await;
    }

    pub async fn chord(&mut self, code: KeyCode, mods: KeyModifiers) {
        self.send_event(Event::Key(KeyEvent::new(code, mods))).await;
    }

    pub async fn type_text(&mut self, text: &str) {
        for ch in text.chars() {
            self.send_event(Event::Key(KeyEvent::new(
                KeyCode::Char(ch),
                KeyModifiers::NONE,
            )))
            .await;
        }
    }

    pub async fn paste(&mut self, text: &str) {
        self.send_event(Event::Paste(text.to_owned())).await;
    }

    pub async fn resize(&mut self, w: u16, h: u16) {
        self.terminal.backend_mut().resize(w, h);
        self.send_event(Event::Resize(w, h)).await;
    }

    async fn send_event(&mut self, event: Event) {
        crate::events::handle_one_event(
            event,
            &mut self.app,
            self.bg_tx.clone(),
            self.token_tx.clone(),
        )
        .await;
        self.render();
    }

    /// Drains every currently-ready channel event through the real handlers, runs one
    /// periodic-tasks pass, then redraws. Mirrors the loop's non-blocking arms.
    pub async fn pump(&mut self) {
        loop {
            let mut progressed = false;
            while let Ok(tok) = self.token_rx.try_recv() {
                commands::handle_stream_token(tok, &mut self.app, self.token_tx.clone())
                    .await
                    .expect("handle_stream_token");
                progressed = true;
            }
            while let Ok(ev) = self.bg_rx.try_recv() {
                commands::handle_background_event(ev, &mut self.app);
                progressed = true;
            }
            while let Ok(u) = self.tokenizer_rx.try_recv() {
                commands::handle_background_event(
                    BackgroundEvent::TokenCountReady(u),
                    &mut self.app,
                );
                progressed = true;
            }
            commands::run_periodic_tasks(&mut self.app, self.token_tx.clone()).await;
            if !progressed {
                break;
            }
        }
        self.render();
    }

    /// Simulates elapsed wall-clock by moving the app's deadline `Instant`s into the
    /// past, then running the periodic tasks that fire on those deadlines, then redrawing.
    pub async fn advance(&mut self, by: Duration) {
        let past = std::time::Instant::now()
            .checked_sub(by)
            .unwrap_or_else(std::time::Instant::now);
        if let Some(msg) = self.app.status_message.as_mut()
            && by >= crate::types::STATUS_DURATION
        {
            msg.expires = past;
        }
        if self.app.pending_save_deadline.is_some() {
            self.app.pending_save_deadline = Some(past);
        }
        self.app.sidebar_age_refresh_at = past;
        commands::run_periodic_tasks(&mut self.app, self.token_tx.clone()).await;
        self.render();
    }

    pub fn observe(&self) -> Observation {
        observe::observe(&self.app)
    }

    pub fn screen_raw(&self) -> String {
        buffer_to_string(self.terminal.backend().buffer())
    }

    /// The rendered screen as text.
    pub fn screen(&self) -> String {
        self.screen_raw()
    }
}

fn buffer_to_string(buf: &ratatui::buffer::Buffer) -> String {
    let area = buf.area();
    let mut out = String::new();
    for y in 0..area.height {
        for x in 0..area.width {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}
