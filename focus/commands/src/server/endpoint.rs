use tonic::transport::Server;

use focus_formats::storage;

#[derive(Debug)]
pub struct Endpoint {}

#[tonic::async_trait]
impl storage::content_storage_server::ContentStorage for Endpoint {
    async fn retrieve_content(
        &self,
        _request: tonic::Request<storage::retrieve_content::Request>,
    ) -> Result<tonic::Response<storage::retrieve_content::Response>, tonic::Status> {
        todo!("Implement");
    }

    async fn store_content(
        &self,
        _request: tonic::Request<storage::store_content::Request>,
    ) -> Result<tonic::Response<storage::store_content::Response>, tonic::Status> {
        todo!("Implement");
    }
}

#[allow(dead_code)]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use storage::content_storage_server::*;

    let addr = "[::1]:10000".parse().unwrap();

    println!("Server listening on: {}", addr);

    let endpoint = Endpoint {};
    let svc = ContentStorageServer::new(endpoint);

    Server::builder().add_service(svc).serve(addr).await?;

    Ok(())
}
