use crate::{SampleRequest, SourceReader};
use lens_contract::{
    activity::{ActivityAvailability, ActivitySelection},
    investigations::Scope,
    worker::{Execution, ExecutionContent, Sample},
};
use lens_server::activity::{ActivityReadError, ActivityReader, PreviewWindow};

fn failure(error: crate::Error) -> ActivityReadError {
    ActivityReadError(Box::new(error))
}

impl ActivityReader for SourceReader {
    async fn availability(&self, scope: &Scope) -> Result<ActivityAvailability, ActivityReadError> {
        let row = SourceReader::availability(self, scope)
            .await
            .map_err(failure)?;
        Ok(ActivityAvailability {
            traces: row.traces != 0,
            requests: row.requests != 0,
        })
    }

    async fn agents(&self, scope: &Scope) -> Result<Vec<String>, ActivityReadError> {
        SourceReader::agents(self, scope).await.map_err(failure)
    }

    async fn preview(
        &self,
        scope: &Scope,
        selection: &ActivitySelection,
        window: PreviewWindow,
    ) -> Result<Sample, ActivityReadError> {
        self.sample(
            scope,
            SampleRequest {
                selection,
                start: window.start,
                end: window.end,
                offset: window.offset,
                page_size: 100,
                preview: true,
                cursor: "",
            },
        )
        .await
        .map_err(failure)
    }

    async fn content(
        &self,
        scope: &Scope,
        execution: &Execution,
        cursor: &str,
        offset: u32,
    ) -> Result<ExecutionContent, ActivityReadError> {
        SourceReader::content(self, scope, execution, cursor, Some(offset))
            .await
            .map_err(failure)
    }
}
