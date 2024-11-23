pub mod error;
pub mod executor;
pub mod jobs;
pub mod mutation;
pub mod query;

pub type ServiceResult<T = ()> = Result<T, error::ServiceError>;
