use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tokio::sync::mpsc;

use crate::types::BackgroundEvent;
use crate::{BuildParams, SummarizerParams};
use libllm_core::config::{Auth, CliOverrides};
use libllm_core::preset::InstructPreset;
use libllm_core::sampling::SamplingParams;
use libllm_core::session::{SaveMode, Session};
use libllm_protocol::client::{ApiClient, StreamToken};
use libllm_protocol::tokenizer::TokenCountUpdate;

use super::Harness;
use super::mock_api::MockApi;

pub enum DbChoice {
    None,
    Temp,
}

pub enum ApiChoice {
    None,
    Mock,
}

pub struct HarnessBuilder {
    size: (u16, u16),
    db: DbChoice,
    api: ApiChoice,
    overrides: CliOverrides,
}

impl HarnessBuilder {
    pub(crate) fn new() -> Self {
        Self {
            size: (100, 30),
            db: DbChoice::None,
            api: ApiChoice::None,
            overrides: CliOverrides::default(),
        }
    }

    pub fn size(mut self, w: u16, h: u16) -> Self {
        self.size = (w, h);
        self
    }

    pub fn no_db(mut self) -> Self {
        self.db = DbChoice::None;
        self
    }

    pub fn temp_db(mut self) -> Self {
        self.db = DbChoice::Temp;
        self
    }

    pub fn no_api(mut self) -> Self {
        self.api = ApiChoice::None;
        self
    }

    pub fn mock_api(mut self) -> Self {
        self.api = ApiChoice::Mock;
        self
    }

    pub fn overrides(mut self, o: CliOverrides) -> Self {
        self.overrides = o;
        self
    }

    /// Builds the harness. Must be called inside a tokio runtime. The caller owns the
    /// `Session` and lends it for the harness lifetime.
    pub async fn build(self, session: &mut Session) -> anyhow::Result<Harness<'_>> {
        let (token_tx, token_rx) = mpsc::channel::<StreamToken>(256);
        let (bg_tx, bg_rx) = mpsc::channel::<BackgroundEvent>(64);
        let (tokenizer_tx, tokenizer_rx) = mpsc::channel::<TokenCountUpdate>(64);

        let is_mock = matches!(self.api, ApiChoice::Mock);
        let (client, mock) = match self.api {
            ApiChoice::None => (
                ApiClient::new("http://127.0.0.1:1", false, Auth::None),
                None,
            ),
            ApiChoice::Mock => {
                let mock = MockApi::start().await;
                let client = ApiClient::new(&mock.base_url(), false, Auth::None);
                (client, Some(mock))
            }
        };

        let (db, summarizer_params, tempdir) = match self.db {
            DbChoice::None => (
                None,
                SummarizerParams {
                    db_path: None,
                    derived_key: None,
                },
                None,
            ),
            DbChoice::Temp => {
                let dir = tempfile::TempDir::new()?;
                let db_path = dir.path().join("test.db");
                let db = libllm_storage::db::Database::open(&db_path, None)?;
                db.ensure_builtin_prompts()?;
                let summarizer_params = SummarizerParams {
                    db_path: Some(db_path),
                    derived_key: None,
                };
                (Some(db), summarizer_params, Some(dir))
            }
        };

        let terminal = Terminal::new(TestBackend::new(self.size.0, self.size.1))?;

        let app = crate::types::App::build(BuildParams {
            client,
            session,
            save_mode: SaveMode::None,
            db,
            instruct_preset: InstructPreset::default(),
            sampling: SamplingParams::default(),
            cli_overrides: self.overrides,
            summarizer_params,
            version_status: "test",
            tokenizer_tx: tokenizer_tx.clone(),
            bg_tx: bg_tx.clone(),
        })?;

        let mut harness = Harness {
            app,
            terminal,
            token_tx,
            token_rx,
            bg_tx,
            bg_rx,
            tokenizer_rx,
            _tempdir: tempdir,
            mock,
        };

        // When the mock API is active, pre-seed the model name so stream_preflight
        // does not block the first completion request with "Connecting to API server...".
        // In the real app this arrives asynchronously via BackgroundEvent::ModelFetched
        // from spawn_startup_probes, which the harness intentionally omits.
        if is_mock {
            harness.app.model_name = Some("test-model".to_owned());
        }

        harness.render();

        Ok(harness)
    }
}
