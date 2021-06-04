use std::env;
use std::path::Path;

use futures::TryFutureExt;
#[cfg(unix)]
use tokio::net::UnixListener;
use tonic::transport::Server;

use focus_formats::storage;
use internals::storage::rocks::{Keygen, Storage};
use storage::{get_inline, store_content, write_to_file, Wants};
use storage::content_storage_server::ContentStorage;

use crate::objectstore::ObjectStore;
use rocksdb::perf::PerfMetric::WritePreAndPostProcessTime;

use anyhow::Context;
use internals::error::AppError;

#[cfg(unix)]
mod unix;

#[derive(Debug)]
pub struct Endpoint {
    storage: ObjectStore
}

impl Endpoint {
    fn get_inline_result(
        &self,
        req: tonic::Request<get_inline::Request>
    ) -> Result<tonic::Response<get_inline::Response>, AppError> {
        let msg = req.get_ref();

        let wants = Wants::from_i32(msg.wants).context("wants enum value")?;
        let ident = msg.content_identifier.as_ref().context("must provide content_identifier")?;
        let hash = ident.hash.as_ref().context("object hash must be non nil")?;

        self.storage.get_inline(hash, wants)
            .map(|rep| tonic::Response::new(rep))
    }
}

const GIT_DIR: &str = "GIT_DIR";
const SOCKET_PATH: &str = "GIT_STORAGE_SOCKET_PATH";


fn get_env(k: &str) -> Option<String> {
    match env::var(k)  {
        Ok(v) => Some(v),
        Err(e) => {
            println!("got error {} accessing {} var", e, SOCKET_PATH);
            None
        }
    }
}


#[tonic::async_trait]
impl ContentStorage for Endpoint {
    async fn store_content(
        &self,
        _request: tonic::Request<store_content::Request>,
    ) -> Result<tonic::Response<store_content::Response>, tonic::Status> {
        todo!("not implemented!")
    }

    async fn write_to_file(
        &self,
        _request: tonic::Request<write_to_file::Request>,
    ) -> Result<tonic::Response<write_to_file::Response>, tonic::Status> {
        todo!("not implemented!")
    }

    async fn get_inline(
        &self,
        req: tonic::Request<get_inline::Request>,
    ) -> Result<tonic::Response<get_inline::Response>, tonic::Status> {
        self.get_inline_result(req)
            .map_err(|err| tonic::Status::internal(err.to_string())) }
}



#[allow(dead_code)]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use storage::content_storage_server::*;

    let git_dir = get_env(GIT_DIR).expect("GIT_DIR env var must be set");
    let git_dir_path = Path::new(git_dir.as_str());
    let storage_dir = git_dir_path.join("storage");
    let db_path = storage_dir.join("db");
    let sock_path = storage_dir.join("server.sock");

    tokio::fs::create_dir_all(storage_dir).await?;

    let incoming = {
        let uds = UnixListener::bind(sock_path.to_str().unwrap())?;
        async_stream::stream! {
            while let item = uds.accept().map_ok(|(st, _)| unix::UnixStream(st)).await {
                yield item;
            }
        }
    };

    println!("Server listening on: {}", sock_path.display());

    let storage = Storage::new(db_path.as_path())?;

    let keygen = Keygen::default();

    let endpoint = Endpoint { storage: ObjectStore::new(storage, keygen) };
    let svc = ContentStorageServer::new(endpoint);

    Server::builder()
        .add_service(svc)
        .serve_with_incoming(incoming)
        .await?;

    Ok(())
}
