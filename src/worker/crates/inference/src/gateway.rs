use url::Url;

use crate::Error;

pub const LENS_AGENT: &str = "lens-signals";

#[derive(Clone)]
pub struct GatewayBase(Url);

impl GatewayBase {
    pub fn new(base: Url) -> Result<Self, Error> {
        if !matches!(base.scheme(), "http" | "https")
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err(Error::GatewayConfiguration);
        }
        Ok(Self(base))
    }

    pub fn covers(&self, endpoint: &Url) -> bool {
        let prefix = self.0.path().trim_end_matches('/');
        endpoint.origin() == self.0.origin()
            && endpoint.path().starts_with(&format!("{prefix}/"))
            && endpoint.username().is_empty()
            && endpoint.password().is_none()
    }
}
