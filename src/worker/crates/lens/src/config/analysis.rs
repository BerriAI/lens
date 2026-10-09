use lens_analysis::{Deployment, Provider, Secret};
use lens_inference::{ModelCapacity, OutputLimits};
use serde::Deserialize;

use crate::Error;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProviderName {
    Openai,
    Anthropic,
    OpenaiCompatible,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Model {
    name: String,
    model: String,
    provider: ProviderName,
    api_key_env: String,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    input_cost_per_token: Option<f64>,
    #[serde(default)]
    output_cost_per_token: Option<f64>,
    #[serde(default)]
    capacity: ModelCapacity,
    #[serde(default)]
    output_limits: OutputLimits,
}

pub(super) fn read(read: &impl Fn(&str) -> Option<String>) -> Result<Vec<Deployment>, Error> {
    let Some(value) = read("LENS_ANALYSIS_MODELS") else {
        return Ok(vec![]);
    };
    let models: Vec<Model> = serde_json::from_str(&value).map_err(|_| {
        Error::Configuration(
            "LENS_ANALYSIS_MODELS must contain a JSON array of analysis deployments",
        )
    })?;
    models
        .into_iter()
        .map(|model| {
            if model.api_key_env.is_empty()
                || !model
                    .api_key_env
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            {
                return Err(Error::Configuration(
                    "Analysis api_key_env must name a provider credential environment variable",
                ));
            }
            let credential = read(&model.api_key_env)
                .filter(|value| !value.is_empty())
                .ok_or(Error::Configuration(
                    "An analysis deployment's api_key_env is unset or empty",
                ))?;
            let api_base = model
                .api_base
                .as_deref()
                .map(url::Url::parse)
                .transpose()
                .map_err(|_| Error::Configuration("Analysis api_base must be a valid URL"))?;
            Ok(Deployment {
                name: model.name,
                model: model.model,
                provider: match model.provider {
                    ProviderName::Openai => Provider::OpenAi,
                    ProviderName::Anthropic => Provider::Anthropic,
                    ProviderName::OpenaiCompatible => Provider::OpenAiCompatible,
                },
                api_base,
                api_key: Secret::new(credential),
                input_cost_per_token: model.input_cost_per_token,
                output_cost_per_token: model.output_cost_per_token,
                capacity: model.capacity,
                output_limits: model.output_limits,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::read;
    use rstest::rstest;

    #[rstest]
    fn tracing_only_startup_does_not_require_an_analysis_credential() {
        assert!(read(&|_| None).unwrap().is_empty());
    }

    #[rstest]
    #[case::invalid_json("credential-shaped-non-json")]
    #[case::inline_key(
        r#"[{"name":"analysis","model":"example","provider":"openai","api_key":"secret"}]"#
    )]
    #[case::missing_credential(
        r#"[{"name":"analysis","model":"example","provider":"openai","api_key_env":"MISSING"}]"#
    )]
    #[case::invalid_variable(
        r#"[{"name":"analysis","model":"example","provider":"openai","api_key_env":"bad-name"}]"#
    )]
    fn invalid_configuration_is_rejected_without_echoing_input(#[case] value: &str) {
        let error =
            read(&|name| (name == "LENS_ANALYSIS_MODELS").then(|| value.into())).unwrap_err();
        assert!(!error.to_string().contains(value));
    }

    #[rstest]
    fn provider_credentials_are_resolved_and_redacted() {
        let models=read(&|name|match name {
            "LENS_ANALYSIS_MODELS"=>Some(r#"[{"name":"analysis","model":"example","provider":"anthropic","api_key_env":"TEST_PROVIDER_KEY"}]"#.into()),
            "TEST_PROVIDER_KEY"=>Some("private-provider-credential".into()),_=>None,
        }).unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].name, "analysis");
        assert_eq!(models[0].provider, lens_analysis::Provider::Anthropic);
        assert!(!format!("{:?}", models[0]).contains("private-provider-credential"));
    }
}
