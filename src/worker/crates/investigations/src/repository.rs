use crate::RepositoryError;
use lens_contract::{
    investigations::{Lens, Scope},
    worker::Job,
};
use std::future::Future;

pub trait LensRepository: Send + Sync {
    fn lenses(
        &self,
        scope: &Scope,
    ) -> impl Future<Output = Result<Vec<Lens>, RepositoryError>> + Send;

    fn get(
        &self,
        lens_id: &str,
    ) -> impl Future<Output = Result<Option<Lens>, RepositoryError>> + Send;

    fn create(&self, lens: &Lens) -> impl Future<Output = Result<Lens, RepositoryError>> + Send;

    fn replace(
        &self,
        expected: &Lens,
        candidate: &Lens,
    ) -> impl Future<Output = Result<Lens, RepositoryError>> + Send;

    fn jobs(
        &self,
        lens_id: &str,
        offset: u64,
    ) -> impl Future<Output = Result<Vec<Job>, RepositoryError>> + Send;

    fn job(
        &self,
        lens_id: &str,
        job_id: &str,
    ) -> impl Future<Output = Result<Option<Job>, RepositoryError>> + Send;
}
