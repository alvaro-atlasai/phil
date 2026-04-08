mod executor;
mod manifest;
mod mcp;
mod parser;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use manifest::Manifest;

#[derive(Parser)]
#[command(
    name = "any2mcp",
    about = "Turn any CLI tool into an MCP server using local AI",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate a YAML manifest from a CLI tool's --help output
    Init {
        /// The binary to analyze
        binary: String,
        /// Output file (default: stdout)
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Start an MCP server for a CLI tool
    Serve {
        /// The binary to serve (auto-parses --help if no manifest)
        binary: Option<String>,
        /// Path to a YAML manifest file
        #[arg(short, long)]
        manifest: Option<String>,
    },
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("any2mcp: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { binary, output } => {
            let manifest = parser::parse_binary(&binary).await?;
            let yaml = manifest.to_yaml()?;

            match output {
                Some(path) => {
                    manifest.save(&PathBuf::from(&path))?;
                    eprintln!("Manifest written to {path}");
                }
                None => {
                    print!("{yaml}");
                }
            }
        }
        Commands::Serve { binary, manifest: manifest_path } => {
            let manifest = match manifest_path {
                Some(path) => Manifest::load(&PathBuf::from(&path))?,
                None => {
                    let binary = binary.ok_or("either --manifest or a binary name is required")?;
                    eprintln!("No manifest provided, analyzing `{binary}` on the fly...");
                    parser::parse_binary(&binary).await?
                }
            };

            mcp::serve(&manifest)?;
        }
    }

    Ok(())
}
