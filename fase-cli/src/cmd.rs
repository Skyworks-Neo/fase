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
}
