use anyhow::{bail, Context, Result};
use futures_util::FutureExt;
use lazy_static::lazy_static;
use log::{error, info, warn};
use std::ffi::{CStr, CString, OsString};
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::{cell::Cell, collections::HashMap, io::BufReader, path::Path, process::Command};
lazy_static! {
    // static ref CLIENT: Arc<Mutex<Cell<Option<client::Client>>>> = Arc::new(Mutex::new(Cell::new(None)));
}

#[derive(Debug, Clone)]
pub struct DelegateConfig {
    repo_path: PathBuf,
    fifo_path: PathBuf,
    args: OsString,
    hash_raw_bytes: usize,
}

impl DelegateConfig {
    pub(crate) unsafe fn new(
        repo_path: *const libc::c_uchar,
        repo_path_length: libc::size_t,
        fifo_path: *const libc::c_uchar,
        fifo_path_length: libc::size_t,
        args: *const libc::c_uchar,
        args_length: libc::size_t,
        hash_raw_bytes: libc::size_t,
    ) -> Self {
        let to_os_string = |bytes: *const libc::c_uchar, len: usize| -> OsString {
            let byte_string = Vec::from_raw_parts(bytes as *mut libc::c_uchar, len, len);
            OsString::from_vec(byte_string)
        };

        Self {
            repo_path: PathBuf::from(to_os_string(repo_path, repo_path_length)),
            fifo_path: PathBuf::from(to_os_string(fifo_path, fifo_path_length)),
            args: to_os_string(args, args_length),
            hash_raw_bytes,
        }
    }
}

#[allow(dead_code)]
struct DelegateContext {
    config: DelegateConfig,
    client: blockingclient::BlockingClient,
}

impl DelegateContext {
    pub fn new(config: DelegateConfig) -> Result<Self> {
        let client = blockingclient::BlockingClient::connect("http://[::1]:60606")?;

        Ok(Self { config, client })
    }

    pub fn client(&self) -> &blockingclient::BlockingClient {
        &self.client
    }
}

pub(crate) mod blockingclient {
    use focus_formats::{
        parachute::content_digest,
        storage::{content_storage_client::ContentStorageClient, get_inline, ContentDigest},
    };

    use tokio::runtime::{Builder, Runtime};

    type StdError = Box<dyn std::error::Error + Send + Sync + 'static>;
    type Result<T, E = StdError> = ::std::result::Result<T, E>;

    // The order of the fields in this struct is important. They must be ordered
    // such that when `BlockingClient` is dropped the client is dropped
    // before the runtime. Not doing this will result in a deadlock when dropped.
    // Rust drops struct fields in declaration order.
    pub struct BlockingClient {
        client: ContentStorageClient<tonic::transport::Channel>,
        rt: Runtime,
    }

    impl BlockingClient {
        pub fn connect<D>(dst: D) -> Result<Self, tonic::transport::Error>
        where
            D: std::convert::TryInto<tonic::transport::Endpoint>,
            D::Error: Into<StdError>,
        {
            let rt = Builder::new_multi_thread().enable_all().build().unwrap();
            let client = rt.block_on(ContentStorageClient::connect(dst))?;

            Ok(Self { rt, client })
        }

        pub fn say_hello(
            &mut self,
            request: impl tonic::IntoRequest<get_inline::Request>,
        ) -> Result<tonic::Response<get_inline::Response>, tonic::Status> {
            self.rt.block_on(self.client.get_inline(request))
        }
    }
}

fn read_git_config(repo_path: &Path) -> Result<HashMap<String, String>> {
    use std::io::prelude::*;

    let output = Command::new("git")
        .arg("-C")
        .arg(repo_path.as_os_str())
        .arg("config")
        .arg("-l")
        .output()?;
    if output.status.success() {
        bail!("git config failed");
    }

    let mut map = HashMap::<String, String>::new();
    for line in output.stdout.lines() {
        if let Ok(line) = line {
            if let Some((key, value)) = line.split_once('=') {
                map.insert(key.to_owned(), value.to_owned());
            }
        }
    }

    Ok(map)
}

const SERVER_PATH_CONFIG_KEY: &'static str = "twitter.server.path";

fn start_server(config: &DelegateConfig) -> Result<()> {
    let git_config = read_git_config(config.repo_path.as_path()).context("reading git config")?;
    if let Some(value) = git_config.get(SERVER_PATH_CONFIG_KEY) {
        todo!("not implemented");
        // Use the value to initiate the server... Disown the process correctly, etc.
        // FIXME(wilhelm) implement the rest here.
    }

    Ok(())
}

#[allow(unused_variables)]
#[no_mangle]
pub extern "C" fn git_storage_init(
    repo_path: *const libc::c_uchar,
    repo_path_length: libc::size_t,
    fifo_path: *const libc::c_uchar,
    fifo_path_length: libc::size_t,
    args: *const libc::c_uchar,
    args_length: libc::size_t,
    hash_raw_bytes: libc::size_t,
    attachment: *mut *mut libc::c_void, // User attachment (will be allocated)
) -> libc::c_int {
    unsafe {
        let config = DelegateConfig::new(
            repo_path,
            repo_path_length,
            fifo_path,
            fifo_path_length,
            args,
            args_length,
            hash_raw_bytes,
        );

        let context: Cell<Option<DelegateContext>> = Cell::new(None);

        // See if the server is running by trying to connect
        if let Ok(ctx) = DelegateContext::new(config.clone()) {
            // Server is running
            context.replace(Some(ctx));
        } else {
            // Server is not running.
            start_server(&config);

            if let Ok(ctx) = DelegateContext::new(config.clone()) {
                // Server is running
                context.replace(Some(ctx));
            } else {
                // Server is not running.
                error!(
                    "Could not start server for repo in {}",
                    &config.repo_path.display()
                );
            }
        }

        *attachment =
            Box::into_raw(Box::new(context.take().expect("missing context"))) as *mut libc::c_void;

        0
    }
}

#[allow(unused_variables)]
#[no_mangle]
pub extern "C" fn git_storage_shutdown(
    attachment: *mut libc::c_void, // User attachment (will be allocated)
) -> libc::c_int {
    let attachment =
        unsafe { Box::<DelegateContext>::from_raw(attachment as *mut DelegateContext) };

    -1
}

#[allow(unused_variables)]
#[no_mangle]
pub extern "C" fn git_storage_fetch_object(
    // User attachment
    attachment: *mut libc::c_void,

    // Object ID, whose length corresponds to |hash_raw_bytes|
    oid: *const libc::c_uchar,

    // If non-zero, the delegate should attempt to return a copy of the object as a location on disk.
    reply_on_disk: libc::c_int,

    // For memory-backed replies, the delegate will set this to a buffer containing the response. It will be freed by Git.
    memory_reply_buf: *mut *mut libc::c_uchar,

    // If `reply_on_disk` is nonzero, the delegate will set this to a NUL-terminated string containing the path to the file on disk containing the response.
    disk_reply_path: *mut *mut libc::c_char,

    // Length of the disk_reply_path string.
    disk_reply_path_len: *mut libc::size_t,

    // Location of the header in the indicated file or buffer
    header_offset: *mut libc::off_t,

    // Length of the header
    header_length: *mut libc::size_t,

    // Location of the content in the indicated file or buffer
    content_offset: *mut libc::off_t,

    // Length of the content
    content_length: *mut libc::size_t,

    // Access time
    atime: *mut libc::time_t,

    // Modified time
    mtime: *mut libc::time_t,
) -> libc::c_int {
    let attachment =
        unsafe { Box::<DelegateContext>::from_raw(attachment as *mut DelegateContext) };
    -1
}

#[allow(unused_variables)]
#[no_mangle]
pub extern "C" fn git_storage_size_object(
    attachment: *mut libc::c_void, // User attachment
    oid: *const libc::c_uchar,     // Object ID, whose length corresponds to |hash_raw_bytes|
    size: *mut libc::size_t,       // Size of the object
    atime: *mut libc::time_t,      // Access time
    mtime: *mut libc::time_t,      // Modified time
) -> libc::c_int {
    let attachment =
        unsafe { Box::<DelegateContext>::from_raw(attachment as *mut DelegateContext) };
    -1
}

#[allow(unused_variables)]
#[no_mangle]
pub extern "C" fn git_storage_write_object(
    attachment: *mut libc::c_void, // User attachment
    oid: *const libc::c_uchar,     // Object ID, whose length corresponds to |hash_raw_bytes|
    header: *const libc::c_uchar,  // The header
    header_length: libc::size_t,   // How long the header is
    body: *const libc::c_uchar,    // The body
    body_length: libc::size_t,     // How long the body is
    mtime: libc::time_t,           // Modified time
) -> libc::c_int {
    let attachment =
        unsafe { Box::<DelegateContext>::from_raw(attachment as *mut DelegateContext) };
    -1
}

// TODO: Glue in terms of a trait
// TODO: Intermediate structs (duh)
