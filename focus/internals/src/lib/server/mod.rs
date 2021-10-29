use focus_formats::svc;
use tonic::transport::Server;

#[derive(Debug, Default)]
pub struct WorkbenchesService {}

#[tonic::async_trait]
impl svc::workbenches_server::Workbenches for WorkbenchesService {
    async fn create(
        &self,
        _request: tonic::Request<svc::create::Request>,
    ) -> Result<tonic::Response<svc::create::Response>, tonic::Status> {
        todo!()
    }

    async fn dispose(
        &self,
        _request: tonic::Request<svc::dispose::Request>,
    ) -> Result<tonic::Response<svc::dispose::Response>, tonic::Status> {
        todo!()
    }
}

#[allow(dead_code)]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use svc::workbenches_server::WorkbenchesServer;

    let addr = "[::1]:50051".parse()?;
    let service = WorkbenchesService::default();

    Server::builder()
        .add_service(WorkbenchesServer::new(service))
        .serve(addr)
        .await?;

    Ok(())
}
