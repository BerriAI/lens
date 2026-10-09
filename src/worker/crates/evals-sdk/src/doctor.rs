use serde::Serialize;
use uuid::Uuid;

use crate::{Error, client::Client, model::EvalSpec, setup::Settings};

#[derive(Serialize)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

impl Check {
    pub fn failure(name: &str, error: Error) -> Self {
        Self {
            name: name.to_owned(),
            ok: false,
            detail: format!("{} {}", error, error.hint()).trim().to_owned(),
        }
    }
    pub fn success(name: &str, detail: String) -> Self {
        Self {
            name: name.to_owned(),
            ok: true,
            detail,
        }
    }
}

pub async fn diagnose(client: &Client, settings: &Settings, specs: &[EvalSpec]) -> Vec<Check> {
    let mut checks = vec![Check::success("agent", settings.project.clone())];
    for spec in specs {
        let check = async {
            spec.validate()?;
            let resolved = client.resolve(&spec.data).await?;
            let cases = spec.select(&client.cases(&resolved).await?)?;
            Ok(Check::success(
                &spec.name,
                format!(
                    "{}@{}: {} included cases, {} trials",
                    resolved.name,
                    resolved.revision,
                    cases.len(),
                    cases.len() * spec.trials
                ),
            ))
        }
        .await;
        checks.push(match check {
            Ok(check) => check,
            Err(error) => Check::failure(&spec.name, error),
        });
    }
    let probe = Uuid::new_v4().simple().to_string();
    checks.push(match client.get(&probe, false).await {
        Err(Error::Api { status: 404, code }) if code == "run_not_found" => {
            Check::success("eval API", "Contract v1 run lookup is available".to_owned())
        }
        Ok(_) => Check::success("eval API", "Contract v1 run lookup is available".to_owned()),
        Err(error) => Check::failure("eval API", error),
    });
    checks
}
