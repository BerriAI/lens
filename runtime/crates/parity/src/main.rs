use std::path::PathBuf;

use clap::{Parser, Subcommand};
use lens_parity::scenarios::{auth_scenarios, dataset_scenarios};
use lens_parity::{Tokens, record_scenarios, replay_fixtures};
use url::Url;

#[derive(Parser)]
struct Arguments {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Record(Options),
    Replay(Options),
}

#[derive(clap::Args)]
struct Options {
    #[arg(long)]
    base_url: Url,
    #[arg(long, default_value_os_t = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures"))]
    fixtures: PathBuf,
    #[arg(long, default_value = "parity-admin-token")]
    admin_token: String,
    #[arg(long, default_value = "parity-gateway-secret")]
    gateway_secret: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = Arguments::parse();
    match arguments.command {
        Command::Record(options) => {
            let scenarios = [auth_scenarios(), dataset_scenarios()].concat();
            let count = record_scenarios(
                &options.base_url,
                &options.fixtures,
                &scenarios,
                &Tokens::new(options.admin_token, options.gateway_secret),
            )
            .await?;
            println!(
                "Recorded {count} fixtures in {}",
                options.fixtures.display()
            );
        }
        Command::Replay(options) => {
            let report = replay_fixtures(
                &options.base_url,
                &options.fixtures,
                &Tokens::new(options.admin_token, options.gateway_secret),
            )
            .await?;
            for failure in &report.failures {
                println!("{}", failure.file_name);
                for mismatch in &failure.mismatches {
                    println!(
                        "{}: expected {}, got {}",
                        mismatch.path, mismatch.expected, mismatch.actual
                    );
                }
                println!("{}", failure.diff);
            }
            println!("{} passed, {} failed", report.passed, report.failed());
            if report.failed() > 0 {
                std::process::exit(1);
            }
        }
    }
    Ok(())
}
