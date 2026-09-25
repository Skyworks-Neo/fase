use clap::{Parser, Subcommand};

#[derive(Parser)]
struct Args {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Crd,
    Run,
    Input,
    Output,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    tracing_subscriber::fmt::init();
    match Args::parse().command {
        Command::Crd => print!("{}", fase_api::crds_yaml()?),
        Command::Run => fase_controller::controller::run()
            .await
            .map_err(|error| format!("controller: {error}"))?,
        Command::Input => fase_controller::transfer::run_input()
            .await
            .map_err(|error| format!("input: {error}"))?,
        Command::Output => fase_controller::transfer::run_output()
            .await
            .map_err(|error| format!("output: {error}"))?,
    }
    Ok(())
}
