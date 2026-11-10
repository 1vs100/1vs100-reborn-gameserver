//! `question-import`: fetches trivia questions from a source (OpenTriviaDB first) and writes the
//! question bank file the game server imports (`POST /admin/questions/import`). A crate of its own
//! so the server binary does not link an HTTP client.

pub mod bank;
pub mod categories;
pub mod fit;
pub mod source;
pub mod sources;
