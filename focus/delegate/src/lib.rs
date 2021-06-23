use anyhow::Result;
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;

pub(crate) struct DelegateConfig {
    repo_path: PathBuf,
    fifo_path: PathBuf,
    args: OsString,
    hash_raw_bytes: usize,
}

impl DelegateConfig {
    pub(crate) fn new(
        repo_path: *const libc::c_uchar,
        repo_path_length: libc::size_t,
        fifo_path: *const libc::c_uchar,
        fifo_path_length: libc::size_t,
        args: *const libc::c_uchar,
        args_length: libc::size_t,
        hash_raw_bytes: libc::size_t,
    ) -> Result<Self> {
        let repo_path_slice = unsafe { std::slice::from_raw_parts(repo_path, repo_path_length) };
        let fifo_path_slice = unsafe { std::slice::from_raw_parts(fifo_path, fifo_path_length) };
        let args_slice = unsafe { std::slice::from_raw_parts(args, args_length) };

        Ok(
            Self{
                repo_path: PathBuf::from(OsString::from(repo_path_slice)),
                fifo_path: PathBuf::from(OsString::from(fifo_path_slice)),
                args: OsString::from(args_slice),
                hash_raw_bytes,
            }
        )
    }
}

#[allow(dead_code)]
struct Context {
    config: DelegateConfig,
}

impl Context {
    pub(crate) fn new(config: DelegateConfig) -> Result<Self> {
        Ok(Self{
            config
        })
    }
}

pub(crate) mod client {
    use storage::content_storage_client::ContentStorageClient;
    use anyhow::Result;
    use std::path::{PathBuf, Path};
    use std::ffi::OsString;
    use crate::DelegateConfig;
    use tokio::net::UnixStream;
    use tonic::transport::{Endpoint, Uri};
    use tower::service_fn;

    pub struct Client<'client> {
     config: &'client DelegateConfig
    }

    impl<'client> Client {
        pub fn new(config: &'client DelegateConfig) -> Result<Client> {
            Ok(Self{config})
        }

        // pub fn ensure_server_started(&self) -> Result<()> {
        //     todo!("implement me")
        //     if let Some(metadata) = std::fs::metadata(self.socket_path()) {
        //         // The socket exists. Try to ping it.
        //         let connection = ContentStorageClient::new
        //     } else {
        //         // Stat failed.
        //     }
        //
        //     Ok(())
        // }

        pub fn start_server(&self) -> Result<()> {
            todo!("Impl")
        }

        async pub fn connect(&self) -> Result<Endpoint> {
            let channel = Endpoint::try_from("http://[::]:50051")?
                .connect_with_connector(service_fn(|_: Uri| {
                    let path = self.socket_path()?;

                    // Connect to a Uds socket
                    UnixStream::connect(path)
                }))
                .await?;

            //
            // let channel = Endpoint::try_from("http://[::]:50051")?
            //     .connect_with_connector(service_fn(|_: Uri| {
            //         let path = "/tmp/tonic/helloworld";
            //
            //         // Connect to a Uds socket
            //         UnixStream::connect(path)
            //     }))
            //     .await?;
            //
            // let mut client = GreeterClient::new(channel);
            //
            // let request = tonic::Request::new(HelloRequest {
            //     name: "Tonic".into(),
            // });
            //
            // let response = client.say_hello(request).await?;
            //
            // println!("RESPONSE={:?}", response);
            //
            // Ok(())

        }

        pub fn git_dir(&self) -> Result<PathBuf> {
            Ok(self.config.repo_path.to_owned())
        }

        pub fn objects_dir(&self) -> Result<PathBuf> {
            Ok(self.git_dir()?.join("objects"))
        }

        pub fn database_dir(&self) -> Result<PathBuf> {
            Ok(self.objects_dir()?.join("database"))
        }

        pub fn socket_path(&self) -> Result<PathBuf> {
            Ok(self.database_dir()?.join("SOCKET"))
        }

        // pub fn server_pidfile(&self) -> Result<PathBuf> {
        //
        // }
    }
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
        let to_path = |bytes: *const libc::c_uchar, len: usize| -> PathBuf {
            let byte_string = Vec::from_raw_parts(bytes as *mut libc::c_uchar, len, len);
            let os_string: OsString = OsStringExt::from_vec(byte_string);
            PathBuf::from(os_string)
        };

        *attachment = Box::into_raw(Box::new(Context {
            repo_path: to_path(repo_path, repo_path_length),
            fifo_path: to_path(fifo_path, fifo_path_length),
            args: String::from_raw_parts(args as *mut libc::c_uchar, args_length, args_length),
            hash_raw_bytes,
        })) as *mut libc::c_void;
    }

    0
}

#[allow(unused_variables)]
#[no_mangle]
pub extern "C" fn git_storage_shutdown(
    attachment: *mut libc::c_void, // User attachment (will be allocated)
) -> libc::c_int {
    let attachment = unsafe { Box::<Context>::from_raw(attachment as *mut Context) };

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
    let attachment = unsafe { Box::<Context>::from_raw(attachment as *mut Context) };
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
    let attachment = unsafe { Box::<Context>::from_raw(attachment as *mut Context) };
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
    let attachment = unsafe { Box::<Context>::from_raw(attachment as *mut Context) };
    -1
}

// TODO: Glue in terms of a trait
// TODO: Intermediate structs (duh)
