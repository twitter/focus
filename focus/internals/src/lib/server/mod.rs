use std::{path::Path, sync::Arc};

use focus_formats::svc;
use tonic::transport::Server;

use crate::app::App;

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
pub fn run(listen_address: &str, _repos: &Path, _app: Arc<App>) -> Result<(), Box<dyn std::error::Error>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            use svc::workbenches_server::WorkbenchesServer;
            let addr = listen_address.parse()?;
            let service = WorkbenchesService::default();

            Server::builder()
                .add_service(WorkbenchesServer::new(service))
                .serve(addr)
                .await?;

            Ok(())
        })
}
