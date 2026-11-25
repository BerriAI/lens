use crate::{Error, evidence::Workspace, wire};
use serde_json::Value;
use tokio::io::AsyncWriteExt;

pub async fn execute(workspace: &Workspace, request: &wire::PythonRequest) -> Result<Value, Error> {
    let input = tempfile::NamedTempFile::new()?;
    let mut file = tokio::fs::File::create(input.path()).await?;
    file.write_all(b"{\"code\":").await?;
    file.write_all(&serde_json::to_vec(&request.code)?).await?;
    file.write_all(b",\"data\":").await?;
    workspace.python_input(request, &mut file).await?;
    file.write_all(b"}").await?;
    file.flush().await?;
    drop(file);
    Ok(lens_agents::sandbox::execute(input).await?)
}
