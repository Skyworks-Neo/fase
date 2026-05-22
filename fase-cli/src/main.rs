mod build;
mod cmd;
mod kustomize;

use std::path::{Path, PathBuf};

#[compio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use clap::Parser;
    let cmd = cmd::Cmd::parse();
    cmd.run().await
}
