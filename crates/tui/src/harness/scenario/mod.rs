mod parser;

pub use parser::parse;

#[derive(Debug, Clone, PartialEq)]
pub enum DbSetup {
    None,
    Temp,
    Encrypted(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ApiSetup {
    None,
    Mock,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Setup {
    pub size: (u16, u16),
    pub db: DbSetup,
    pub api: ApiSetup,
    pub overrides: Vec<String>,
    pub seed: Option<String>,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            size: (100, 30),
            db: DbSetup::None,
            api: ApiSetup::None,
            overrides: Vec::new(),
            seed: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Matcher {
    Eq(String),
    Contains(String),
    Null,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Key(String),
    Type(String),
    Paste(String),
    Resize(u16, u16),
    Pump,
    Advance(std::time::Duration),
    EnqueueCompletion(Vec<String>),
    EnqueueError(String),
    ExpectScreenContains(String),
    ExpectScreenExcludes(String),
    ExpectLine { n: usize, matcher: Matcher },
    ExpectState { probe: String, matcher: Matcher },
    Snapshot(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scenario {
    pub setup: Setup,
    pub steps: Vec<Step>,
}
