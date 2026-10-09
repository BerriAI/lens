use lens_inference::{
    DeploymentEstimate, Error, ModelCapacity, OutputLimits, Prices, deployment_exceeds_context,
    deployment_prices, exceeds_context, output_tokens, quote,
};
use rstest::{fixture, rstest};
use serde_json::json;
use std::num::NonZeroU64;

#[fixture]
fn prices() -> Prices {
    Prices {
        input_cost_per_token: 0.001,
        output_cost_per_token: 0.002,
        ..Prices::default()
    }
}

fn nonzero(value: u64) -> Option<NonZeroU64> {
    NonZeroU64::new(value)
}

#[rstest]
#[case::missing(json!({}))]
#[case::null(json!({
    "input_cost_per_token_above_200k_tokens":null,
    "output_cost_per_token_above_200k_tokens":null,
    "input_cost_per_token_above_128k_tokens":null,
    "output_cost_per_token_above_128k_tokens":null,
    "input_cost_per_token_above_272k_tokens":null,
    "output_cost_per_token_above_272k_tokens":null,
    "cache_creation_input_token_cost":null,
    "cache_creation_input_token_cost_above_200k_tokens":null,
    "cache_creation_input_token_cost_above_272k_tokens":null
}))]
fn optional_tier_rates_fall_back_to_base(prices: Prices, #[case] extra: serde_json::Value) {
    let mut value = json!({"input_cost_per_token":0.001,"output_cost_per_token":0.002});
    value
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    let parsed: Prices = serde_json::from_value(value).unwrap();
    assert_eq!(parsed, prices);
    assert_eq!(
        quote(
            &[DeploymentEstimate {
                prices: parsed,
                prompt_tokens: 20,
                output_tokens: 30
            }],
            true
        )
        .unwrap(),
        0.08
    );
}

#[rstest]
#[case::custom(Some(0.0), Some(0.0), (0.0,0.0))]
#[case::partial_input(Some(0.4), None, (0.001,0.002))]
#[case::partial_output(None, Some(0.4), (0.001,0.002))]
#[case::catalog(None, None, (0.001,0.002))]
fn complete_explicit_prices_override_catalog(
    prices: Prices,
    #[case] input: Option<f64>,
    #[case] output: Option<f64>,
    #[case] expected: (f64, f64),
) {
    let result = deployment_prices("custom/model", input, output, Some(&prices)).unwrap();
    assert_eq!(
        (result.input_cost_per_token, result.output_cost_per_token),
        expected
    );
}

#[rstest]
fn custom_rates_discard_catalog_tiers(prices: Prices) {
    let catalog = Prices {
        input_cost_per_token_above_200k_tokens: 9.0,
        cache_creation_input_token_cost: 8.0,
        ..prices
    };
    assert_eq!(
        deployment_prices("model", Some(0.001), Some(0.002), Some(&catalog)).unwrap(),
        prices
    );
}

#[rstest]
#[case::unmapped(None, None)]
#[case::partial(Some(0.001), None)]
#[case::negative_input(Some(-0.001),Some(0.002))]
#[case::negative_output(Some(0.001),Some(-0.002))]
#[case::nan_input(Some(f64::NAN), Some(0.002))]
#[case::nan_output(Some(0.001), Some(f64::NAN))]
fn missing_or_invalid_prices_have_configuration_guidance(
    #[case] input: Option<f64>,
    #[case] output: Option<f64>,
) {
    let error = deployment_prices("custom/model", input, output, None).unwrap_err();
    assert!(matches!(error, Error::Pricing { .. }));
    assert_eq!(
        error.to_string(),
        "Pricing is not configured for custom/model. Set input_cost_per_token and output_cost_per_token on its deployment before running an investigation."
    );
}

#[rstest]
#[case::input(-1.0,0.0)]
#[case::output(0.0,-1.0)]
fn invalid_catalog_base_prices_are_rejected(#[case] input: f64, #[case] output: f64) {
    let prices = Prices {
        input_cost_per_token: input,
        output_cost_per_token: output,
        ..Prices::default()
    };
    assert!(matches!(
        deployment_prices("model", None, None, Some(&prices)),
        Err(Error::Pricing { .. })
    ));
}

#[rstest]
#[case::completion(1, 2, 3, 4, 1)]
#[case::legacy(0, 2, 3, 4, 2)]
#[case::metadata(0, 0, 3, 4, 3)]
#[case::catalog(0, 0, 0, 4, 4)]
fn configured_output_capacity_has_stable_precedence(
    #[case] completion: u64,
    #[case] legacy: u64,
    #[case] metadata: u64,
    #[case] catalog: u64,
    #[case] expected: u64,
) {
    let limits = OutputLimits {
        max_completion_tokens: nonzero(completion),
        max_tokens: nonzero(legacy),
        max_output_tokens: nonzero(metadata),
    };
    let catalog = ModelCapacity {
        max_output_tokens: nonzero(catalog),
        ..ModelCapacity::default()
    };
    assert_eq!(
        output_tokens("model", &limits, catalog, None).unwrap(),
        expected
    );
}

#[rstest]
fn missing_output_capacity_requires_operator_metadata() {
    let error = output_tokens(
        "custom/model",
        &OutputLimits::default(),
        ModelCapacity::default(),
        Some(10),
    )
    .unwrap_err();
    assert!(matches!(error, Error::OutputCapacity { .. }));
    assert_eq!(
        error.to_string(),
        "Output capacity is unknown for custom/model. Set model_info.max_output_tokens to the model's supported output capacity or configure max_tokens on its deployment."
    );
}

#[rstest]
#[case::short_prompt(10, 80)]
#[case::fits_exactly(20, 80)]
#[case::room_reduced(60, 40)]
#[case::window_full(100, 0)]
#[case::already_overflowing(101, 80)]
fn shared_context_leaves_room_for_the_entire_prompt(#[case] prompt: u64, #[case] expected: u64) {
    let limits = OutputLimits {
        max_tokens: nonzero(80),
        ..OutputLimits::default()
    };
    let catalog = ModelCapacity {
        max_input_tokens: nonzero(100),
        max_output_tokens: nonzero(100),
    };
    assert_eq!(
        output_tokens("model", &limits, catalog, Some(prompt)).unwrap(),
        expected
    );
}

#[rstest]
#[case::below_output_limit(80, 80)]
#[case::at_output_limit(100, 100)]
#[case::above_output_limit(120, 100)]
fn independent_context_clamps_only_the_output_limit(
    #[case] configured: u64,
    #[case] expected: u64,
) {
    let limits = OutputLimits {
        max_tokens: nonzero(configured),
        ..OutputLimits::default()
    };
    let catalog = ModelCapacity {
        max_input_tokens: nonzero(1000),
        max_output_tokens: nonzero(100),
    };
    assert_eq!(
        output_tokens("model", &limits, catalog, Some(950)).unwrap(),
        expected
    );
}

#[rstest]
#[case::with_prompt(Some(1000))]
#[case::without_prompt(None)]
fn unknown_catalog_preserves_explicit_output_allowance(#[case] prompt: Option<u64>) {
    let limits = OutputLimits {
        max_tokens: nonzero(80),
        ..OutputLimits::default()
    };
    assert_eq!(
        output_tokens("custom/model", &limits, ModelCapacity::default(), prompt).unwrap(),
        80
    );
}

#[rstest]
#[case::below(99, 100, 10, false)]
#[case::equal(100, 100, 10, true)]
#[case::above(101, 100, 10, true)]
#[case::catalog(100, 0, 100, true)]
#[case::unknown(100, 0, 0, false)]
fn context_preflight_uses_deployment_override_and_boundary(
    #[case] prompt: u64,
    #[case] configured: u64,
    #[case] catalog: u64,
    #[case] expected: bool,
) {
    assert_eq!(
        deployment_exceeds_context(prompt, nonzero(configured), nonzero(catalog)),
        expected
    );
}

#[rstest]
#[case::both_small(100, 100, true)]
#[case::one_large(100, 1000, false)]
#[case::first_large(1000, 100, false)]
#[case::unknown(100, 0, false)]
fn preflight_rejects_only_when_every_deployment_is_too_small(
    #[case] first: u64,
    #[case] second: u64,
    #[case] expected: bool,
) {
    let configured = ModelCapacity {
        max_input_tokens: nonzero(first),
        ..ModelCapacity::default()
    };
    let catalog = ModelCapacity {
        max_input_tokens: nonzero(second),
        ..ModelCapacity::default()
    };
    assert_eq!(
        exceeds_context(&[
            (100, configured, ModelCapacity::default()),
            (100, ModelCapacity::default(), catalog)
        ]),
        expected
    );
}

#[rstest]
#[case::input_base("input_cost_per_token", 0.3, true)]
#[case::input_128k("input_cost_per_token_above_128k_tokens", 0.3, true)]
#[case::input_200k("input_cost_per_token_above_200k_tokens", 0.3, true)]
#[case::input_272k("input_cost_per_token_above_272k_tokens", 0.3, true)]
#[case::output_base("output_cost_per_token", 0.6, true)]
#[case::output_128k("output_cost_per_token_above_128k_tokens", 0.6, true)]
#[case::output_200k("output_cost_per_token_above_200k_tokens", 0.6, true)]
#[case::output_272k("output_cost_per_token_above_272k_tokens", 0.6, true)]
#[case::cache_base("cache_creation_input_token_cost", 0.3, true)]
#[case::cache_200k("cache_creation_input_token_cost_above_200k_tokens", 0.3, true)]
#[case::cache_272k("cache_creation_input_token_cost_above_272k_tokens", 0.3, true)]
#[case::legacy_skips_cache("cache_creation_input_token_cost", 0.0, false)]
fn quote_reserves_the_highest_applicable_rate(
    #[case] field: &str,
    #[case] expected: f64,
    #[case] cacheable: bool,
) {
    let mut value = json!({"input_cost_per_token":0.0,"output_cost_per_token":0.0});
    value[field] = json!(0.03);
    let prices = serde_json::from_value(value).unwrap();
    let estimate = quote(
        &[DeploymentEstimate {
            prices,
            prompt_tokens: 10,
            output_tokens: 20,
        }],
        cacheable,
    )
    .unwrap();
    assert!((estimate - expected).abs() < 1e-12);
}

#[rstest]
fn quote_combines_worst_prices_and_prompt_size_with_shared_output_allowance() {
    let first = DeploymentEstimate {
        prices: Prices {
            input_cost_per_token: 0.1,
            output_cost_per_token: 0.2,
            ..Prices::default()
        },
        prompt_tokens: 20,
        output_tokens: 50,
    };
    let second = DeploymentEstimate {
        prices: Prices {
            input_cost_per_token: 0.3,
            output_cost_per_token: 0.1,
            ..Prices::default()
        },
        prompt_tokens: 10,
        output_tokens: 40,
    };
    assert_eq!(
        quote(&[first, second], false).unwrap(),
        20.0 * 0.3 + 40.0 * 0.2
    );
    assert_eq!(
        quote(&[second, first], false).unwrap(),
        quote(&[first, second], false).unwrap()
    );
}

#[rstest]
fn quote_requires_an_available_deployment() {
    let error = quote(&[], false).unwrap_err();
    assert!(matches!(error, Error::ModelUnavailable));
    assert_eq!(error.to_string(), "Analysis model is no longer available");
}
