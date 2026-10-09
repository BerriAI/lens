use std::{sync::Arc, time::Duration};

use lens_migrate::{Error, import, read_source};
use litellm_http::{
    ClientVariant, HttpClientPool, HttpSettings, Resolution, media::PublicDnsResolver,
};
use litellm_storage_clickhouse::{Connection, Storage, state::ClickHouseState};

fn required(name: &'static str) -> Result<String, Error> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.is_empty())
        .ok_or(Error::Configuration(name))
}

fn clickhouse_url(read: impl Fn(&str) -> Option<String>) -> Result<String, Error> {
    if let Some(url) = read("CLICKHOUSE_URL").filter(|url| !url.is_empty()) {
        return Ok(url);
    }
    let host = read("CLICKHOUSE_HOST")
        .filter(|host| !host.is_empty())
        .ok_or(Error::Configuration("CLICKHOUSE_URL or CLICKHOUSE_HOST"))?;
    let mut url = Connection::parse("http://localhost:8123")?.url().clone();
    url.set_host(Some(&host))
        .map_err(|_| Error::Configuration("CLICKHOUSE_HOST"))?;
    let connection = Connection::configured(
        url.as_str(),
        "lens",
        &read("CLICKHOUSE_USER").unwrap_or_else(|| "default".into()),
        &read("CLICKHOUSE_PASSWORD").unwrap_or_default(),
    )?;
    Ok(connection.url().to_string())
}

async fn run() -> Result<(), Error> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments == ["--help"] {
        println!(
            "lens-migrate [--apply --source-stopped]\n\nWithout arguments, validate and print a read-only PostgreSQL migration plan.\nSet LENS_MIGRATION_POSTGRES_URL to the source connection string.\nApplying uses CLICKHOUSE_URL or CLICKHOUSE_HOST/USER/PASSWORD, with CLICKHOUSE_DATABASE (default lens).\nKeep the old and new Lens runtimes stopped until import and verification finish."
        );
        return Ok(());
    }
    let apply = arguments == ["--apply", "--source-stopped"];
    if !arguments.is_empty() && !apply {
        return Err(Error::Configuration(
            "use --apply --source-stopped or --help",
        ));
    }
    let plan = read_source(&required("LENS_MIGRATION_POSTGRES_URL")?)
        .await?
        .plan()?;
    if apply {
        let database = std::env::var("CLICKHOUSE_DATABASE").unwrap_or_else(|_| "lens".into());
        let storage = Storage::new(database, &clickhouse_url(|name| std::env::var(name).ok())?)?;
        let settings = HttpSettings {
            connect_timeout: Duration::from_secs(5),
            ..Default::default()
        };
        let client = HttpClientPool::new(Arc::new(PublicDnsResolver)).client(
            &Resolution::from(&settings).config,
            ClientVariant::NoRedirect,
        )?;
        let state = ClickHouseState::new(client, storage.reader().clone());
        import(&state, &plan, &format!("/lens/{}", storage.database())).await?;
    }
    println!(
        "{}",
        serde_json::to_string(
            &serde_json::json!({"applied":apply,"verified":apply,"plan":plan.report()})
        )?
    );
    Ok(())
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    match run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::clickhouse_url;
    use litellm_storage_clickhouse::Storage;
    use rstest::rstest;

    #[rstest]
    fn explicit_url_retains_its_credentials_and_path() {
        let configured = "https://reader:password@clickhouse.test/proxy?database=old";
        assert_eq!(
            clickhouse_url(|key| Some(
                if key == "CLICKHOUSE_URL" {
                    configured
                } else {
                    "unused"
                }
                .into()
            ))
            .unwrap(),
            configured
        );
    }

    #[rstest]
    #[case::provided(Some("reader"), Some("safe/password"), "reader", "safe%2Fpassword")]
    #[case::defaults(None, None, "default", "")]
    fn compose_settings_preserve_credentials_and_selected_database(
        #[case] user: Option<&str>,
        #[case] password: Option<&str>,
        #[case] expected_user: &str,
        #[case] expected_password: &str,
    ) {
        let url = clickhouse_url(|key| match key {
            "CLICKHOUSE_HOST" => Some("clickhouse".into()),
            "CLICKHOUSE_USER" => user.map(str::to_owned),
            "CLICKHOUSE_PASSWORD" => password.map(str::to_owned),
            _ => None,
        })
        .unwrap();
        let storage = Storage::new("selected".into(), &url).unwrap();
        assert_eq!(storage.reader().url().host_str(), Some("clickhouse"));
        assert_eq!(storage.reader().url().username(), expected_user);
        assert_eq!(
            storage.reader().url().password().unwrap_or_default(),
            expected_password
        );
        assert_eq!(
            storage
                .reader()
                .url()
                .query_pairs()
                .find(|(key, _)| key == "database")
                .unwrap()
                .1,
            "selected"
        );
    }

    #[rstest]
    #[case::missing(None)]
    #[case::empty(Some(""))]
    #[case::invalid(Some("host/path"))]
    fn missing_or_invalid_target_is_rejected(#[case] host: Option<&str>) {
        assert!(
            clickhouse_url(|key| (key == "CLICKHOUSE_HOST")
                .then(|| host.map(str::to_owned))
                .flatten())
            .is_err()
        );
    }
}
