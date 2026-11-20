use lens_inference::GatewayBase;
use rstest::{fixture, rstest};
use url::Url;

#[fixture]
fn gateway() -> GatewayBase {
    GatewayBase::new(Url::parse("https://gateway.test/proxy").unwrap()).unwrap()
}

#[rstest]
fn endpoint_under_the_base_is_covered(gateway: GatewayBase) {
    assert!(gateway.covers(&Url::parse("https://gateway.test/proxy/v1/chat/completions").unwrap()));
}

#[rstest]
#[case::different_host("https://provider.test/proxy/v1/chat/completions")]
#[case::different_port("https://gateway.test:8443/proxy/v1/chat/completions")]
#[case::different_scheme("http://gateway.test/proxy/v1/chat/completions")]
#[case::outside_path("https://gateway.test/v1/chat/completions")]
#[case::prefix_collision("https://gateway.test/proxy-other/v1/chat/completions")]
#[case::credentials("https://user:password@gateway.test/proxy/v1/chat/completions")]
#[case::username_only("https://user@gateway.test/proxy/v1/chat/completions")]
#[case::password_only("https://:password@gateway.test/proxy/v1/chat/completions")]
fn unrelated_endpoint_is_not_covered(gateway: GatewayBase, #[case] endpoint: &str) {
    assert!(!gateway.covers(&Url::parse(endpoint).unwrap()));
}

#[rstest]
#[case::scheme("ftp://gateway.test")]
#[case::username("https://user@gateway.test")]
#[case::password("https://user:password@gateway.test")]
#[case::query("https://gateway.test?key=x")]
#[case::fragment("https://gateway.test#section")]
fn invalid_configuration_is_rejected(#[case] base: &str) {
    assert!(GatewayBase::new(Url::parse(base).unwrap()).is_err());
}
