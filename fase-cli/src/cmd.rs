use super::*;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(version, about)]
pub struct Cmd {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Build {
        #[arg(short, long)]
        kustomize: PathBuf,
    },
    Kustomize {
        path: PathBuf,
        #[arg(short, long, value_enum, default_value = "count")]
        output: kustomize::Output,
    },
}

impl Cmd {
    pub async fn run(self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        match self.command {
            Commands::Build { kustomize } => build::run(kustomize).await,
            Commands::Kustomize { path, output } => kustomize::run(path, output).await,
        }
    }
}
