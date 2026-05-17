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
        kustomize: String,
    },
    Kustomize {
        path: PathBuf,
    },
}

impl Cmd {
    pub async fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        match self.command {
            Commands::Build { kustomize: _ } => Ok(()),
            Commands::Kustomize { path } => kustomize::run(path).await,
        }
    }
}
