//! Runtime execution support for Fase resources.

mod eval;
mod map;

pub use eval::Evaluator;
pub use map::{Artifact, Context, Error, Input, Result, apply};
