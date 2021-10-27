use tonic::{transport::Server, Request, Response, Status};

// use focus::workbench::Workbenches;
// use hello_world::{HelloReply, HelloRequest};

pub mod svc {
    tonic::include_proto!("focus.workbench");
}

#[derive(Debug, Default)]
pub struct WorkbenchesService {}

#[tonic::async_trait]
impl svc::workbenches_server::Workbenches for WorkbenchesService {
    async fn create(
        &self,
        request: tonic::Request<svc::create::Request>,
    ) -> Result<tonic::Response<svc::create::Response>, tonic::Status> {
        let handle = svc::workbench::Handle { identifer: 5 };
        let response = svc::create::Response {
            handle: Some(handle),
        };
        
        Ok(tonic::Response::new(response))
    }

    async fn dispose(
        &self,
        request: tonic::Request<svc::dispose::Request>,
    ) -> Result<tonic::Response<svc::dispose::Response>, tonic::Status> {
        todo!()
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use svc::workbenches_server::WorkbenchesServer;

    let addr = "[::1]:50051".parse()?;
    // let greeter = MyGreeter::default();
    let service = WorkbenchesService::default();

    Server::builder()
        .add_service(WorkbenchesServer::new(service))
        .serve(addr)
        .await?;

    Ok(())
}
