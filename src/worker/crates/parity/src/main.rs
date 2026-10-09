use std::path::PathBuf;

use clap::{Parser, Subcommand};
use lens_parity::scenarios::signal_scenarios;
use lens_parity::scenarios::{
    activity_scenarios, auth_boundary_scenarios, auth_scenarios, dataset_scenarios,
    feedback_scenarios, investigation_scenarios,
};
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
    group: Option<Group>,
    #[arg(long)]
    base_url: Url,
    #[arg(long, default_value_os_t = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures"))]
    fixtures: PathBuf,
    #[arg(long, default_value = "parity-admin-token")]
    admin_token: String,
    #[arg(long, default_value = "parity-gateway-secret")]
    gateway_secret: String,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum Group {
    Auth,
    AuthBoundaries,
    Datasets,
    Investigations,
    Feedback,
    Activity,
    Signals,
}

impl Group {
    fn directory(self) -> &'static str {
        match self {
            Self::Auth => "auth",
            Self::AuthBoundaries => "auth-boundaries",
            Self::Datasets => "datasets",
            Self::Investigations => "investigations",
            Self::Feedback => "feedback",
            Self::Activity => "activity",
            Self::Signals => "signals",
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = Arguments::parse();
    match arguments.command {
        Command::Record(options) => {
            let scenarios = match options.group {
                Some(Group::Auth) => auth_scenarios(),
                Some(Group::AuthBoundaries) => auth_boundary_scenarios(),
                Some(Group::Datasets) => dataset_scenarios(),
                Some(Group::Investigations) => investigation_scenarios(),
                Some(Group::Feedback) => feedback_scenarios(),
                Some(Group::Activity) => activity_scenarios(),
                Some(Group::Signals) => signal_scenarios(),
                None => [
                    auth_scenarios(),
                    auth_boundary_scenarios(),
                    dataset_scenarios(),
                ]
                .concat(),
            };
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
            let fixtures = options
                .group
                .map(|group| options.fixtures.join(group.directory()))
                .unwrap_or(options.fixtures);
            let report = replay_fixtures(
                &options.base_url,
                &fixtures,
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
