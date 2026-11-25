#[tokio::main]
async fn main() -> Result<(), lens_agents::Error> {
    lens_agents::runtime::serve().await
}
