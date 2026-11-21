use std::path::PathBuf;

use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};

pub const ACTION: &str = "BerriAI/lens/src/sdk/action@51651cc61bc3863b524683a34f02732e2717b7b7";

fn positive_count(value: &str) -> Result<usize, &'static str> {
    value
        .parse::<usize>()
        .ok()
        .filter(|count| *count > 0)
        .ok_or("must be at least one")
}

#[derive(Parser)]
#[command(
    name = "lens",
    about = "Evaluate your agent against Lens datasets",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Operation,
}

#[derive(Subcommand, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "kebab-case")]
pub enum Operation {
    Eval {
        #[arg(conflicts_with = "eval_name")]
        path: Option<PathBuf>,
        #[arg(long = "eval", conflicts_with = "eval_name")]
        name: Option<String>,
        #[arg(long = "name")]
        eval_name: Option<String>,
        #[arg(long)]
        ci: bool,
        #[arg(long)]
        json: bool,
    },
    Doctor {
        path: Option<PathBuf>,
        #[arg(long = "eval")]
        name: Option<String>,
        #[arg(long)]
        ci: bool,
        #[arg(long)]
        json: bool,
    },
    Init {
        dataset: Option<String>,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        base_url: Option<String>,
        #[arg(long)]
        task: Option<String>,
        #[arg(long)]
        demo: bool,
        #[arg(long, default_value = ACTION)]
        action_ref: String,
    },
    #[command(about = "Run a sample eval and show the results")]
    Demo {
        #[arg(long, help = "Print the results without animation")]
        fast: bool,
    },
    #[command(about = "Run every task against every harness and model in your config")]
    Run {
        tasks: Option<PathBuf>,
        #[arg(long)]
        config: PathBuf,
        #[arg(long, default_value_t = 16)]
        concurrency: usize,
        #[arg(long, default_value_t = 1, value_parser = positive_count)]
        trials: usize,
        #[arg(long, help = "Print the results without animation")]
        fast: bool,
        #[arg(
            long,
            value_name = "HARNESS/MODEL",
            help = "Only run these targets, repeatable"
        )]
        #[serde(default)]
        only: Vec<String>,
    },
    #[command(about = "Finish an interrupted run")]
    Resume {
        run_id: String,
        #[arg(long, default_value_t = 16)]
        concurrency: usize,
        #[arg(long, help = "Print the results without animation")]
        fast: bool,
    },
    #[command(about = "Show the report for a saved run")]
    Show { run_id: Option<String> },
    DevServer {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 8765)]
        port: u16,
        #[arg(long)]
        dataset_file: Option<PathBuf>,
    },
    #[command(hide = true)]
    Report {
        file: PathBuf,
        #[arg(long)]
        #[serde(default)]
        via_app: bool,
    },
    ReportStart {
        #[arg(long)]
        name: String,
        #[arg(long)]
        pr: Option<u64>,
    },
    ReportFailed {
        #[arg(long)]
        name: String,
        #[arg(long)]
        pr: Option<u64>,
    },
}

pub fn parse(arguments: &[String]) -> serde_json::Value {
    match Cli::try_parse_from(arguments) {
        Ok(cli) => serde_json::to_value(cli.command).expect("CLI serializes"),
        Err(error) => {
            serde_json::json!({"command":"help", "text": error.to_string(), "code": error.exit_code()})
        }
    }
}
