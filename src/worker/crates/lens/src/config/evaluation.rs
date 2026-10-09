use lens_decisions::{Deployment, Provider, Secret};
use serde::Deserialize;

use crate::Error;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Model {
    name: String,
    model: String,
    provider: Provider,
    #[serde(default)]
    api_key_env: Option<String>,
    #[serde(default)]
    api_base: Option<String>,
}

pub(super) fn read(read: &impl Fn(&str) -> Option<String>) -> Result<Vec<Deployment>, Error> {
    let Some(value) = read("LENS_EVALUATION_MODELS") else {
        return Ok(Vec::new());
    };
    let models: Vec<Model> = serde_json::from_str(&value).map_err(|_| {
        Error::Configuration(
            "LENS_EVALUATION_MODELS must contain a JSON array of evaluation deployments",
        )
    })?;
    models.into_iter().map(|model| {
        let api_key = match model.api_key_env {
            Some(name) => {
                if name.is_empty() || !name.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_') { return Err(Error::Configuration("Evaluation api_key_env must name a provider credential environment variable")); }
                Some(Secret::new(read(&name).filter(|value| !value.is_empty()).ok_or(Error::Configuration("An evaluation deployment's api_key_env is unset or empty"))?))
            }
            None if model.provider == Provider::StrandsDecider => None,
            None => return Err(Error::Configuration("Evaluation deployments require api_key_env")),
        };
        let api_base = model.api_base.as_deref().map(url::Url::parse).transpose().map_err(|_| Error::Configuration("Evaluation api_base must be a valid URL"))?;
        Ok(Deployment { name:model.name, model:model.model, provider:model.provider, api_base, api_key })
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::read;
    use rstest::rstest;

    #[rstest]
    fn tracing_only_startup_requires_no_evaluation_credentials() {
        assert!(read(&|_| None).unwrap().is_empty());
    }

    #[rstest]
    #[case::invalid_json("not-json")]
    #[case::inline_secret(
        r#"[{"name":"signals","model":"test","provider":"typesafe","api_key":"secret"}]"#
    )]
    #[case::missing_key(r#"[{"name":"signals","model":"test","provider":"typesafe"}]"#)]
    #[case::empty_key_name(
        r#"[{"name":"signals","model":"test","provider":"typesafe","api_key_env":""}]"#
    )]
    #[case::invalid_key_name(
        r#"[{"name":"signals","model":"test","provider":"typesafe","api_key_env":"bad-name"}]"#
    )]
    #[case::unset_key(
        r#"[{"name":"signals","model":"test","provider":"typesafe","api_key_env":"UNSET"}]"#
    )]
    fn invalid_configuration_does_not_echo_input(#[case] value: &str) {
        let error =
            read(&|name| (name == "LENS_EVALUATION_MODELS").then(|| value.into())).unwrap_err();
        assert!(!error.to_string().contains(value));
    }

    #[rstest]
    fn configured_provider_key_is_resolved_and_redacted() {
        let models=read(&|name|match name {
            "LENS_EVALUATION_MODELS"=>Some(r#"[{"name":"signals","model":"typesafe/jev-latest","provider":"typesafe","api_key_env":"TEST_KEY","api_base":"https://api.example.test/v1"}]"#.into()),
            "TEST_KEY"=>Some("private-test-key".into()),_=>None,
        }).unwrap();
        assert_eq!(models[0].name, "signals");
        assert_eq!(models[0].model, "typesafe/jev-latest");
        assert_eq!(models[0].provider, lens_decisions::Provider::Typesafe);
        assert_eq!(
            models[0].api_base.as_ref().unwrap().as_str(),
            "https://api.example.test/v1"
        );
        assert!(!format!("{:?}", models[0]).contains("private-test-key"));
    }

    #[rstest]
    fn strands_allows_an_unauthenticated_private_endpoint() {
        let models=read(&|name|(name=="LENS_EVALUATION_MODELS").then(||r#"[{"name":"signals","model":"private","provider":"strands_decider","api_base":"http://localhost:9000"}]"#.into())).unwrap();
        assert!(models[0].api_key.is_none());
    }
}
