mod cmd;
mod common;

use common::*;

#[compio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use clap::Parser;
    let cmd = cmd::Cmd::parse();
    Ok(())
}
