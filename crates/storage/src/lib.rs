//! Database persistence layer for LibLLM: schema, migrations, and repositories.

pub mod db;
pub mod error;
pub mod search;

pub use error::DbError;
