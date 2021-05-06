use internals::error::AppError;
use internals::repo::Repos;
use internals::storage::rocks::Storage;
use redis_module::{Context, RedisError, RedisResult, RedisValue};
use std::{os::unix::process, sync::{Arc, Mutex}};
use log::{debug, info, warn};

pub struct Processor {
    repos: Repos,
}

impl Processor {
    pub fn new(repos: Option<Repos>) -> Result<Self, AppError> {
        Ok(Self {
            repos: repos.unwrap_or(Repos::new(None).expect("Configuring repos failed")),
        })
    }

    // Get the Storage object underlying a repo. Use this to retrieve instances holding locks on the repo object itself for a short a time as possible.
    fn get_storage_for_repo(&self, repo_uuid: &String) -> Result<Arc<Storage>, AppError> {
        if let Ok(locked_repos) = self.repos.underlying.lock() {
            if let Some(arc_mutex_repo) = locked_repos.get(repo_uuid) {
                if let Ok(locked_repo) = arc_mutex_repo.lock() {
                    return Ok(locked_repo.storage().clone())
                } else {
                    warn!("Could not obtain the fine 'repo' lock");
                    return Err(AppError::WriteLockFailed())
                }
            } else {
                warn!("Could not find the specified repo.");
                return Err(AppError::Missing())
            }
        } else {
            warn!("Could not obtain the coarse 'repos' lock");
            return Err(AppError::ReadLockFailed())
        }
    }

    pub fn list_repos(&self, _: &Context, args: Vec<String>) -> RedisResult {
        let mut results = Vec::<String>::new();
        if let Ok(locked_repos) = self.repos.underlying.lock() {
            for (k, arc_mutex_repo) in locked_repos.iter() {
                if let Ok(locked_repo) = arc_mutex_repo.lock() {
                    let dir = locked_repo.work_dir()?.expect("Getting work dir failed");
                    results.push(k.to_owned());
                    results.push(dir);
                } else {
                    return Err(RedisError::String(AppError::ReadLockFailed().into()));
                }
            }
        }
        return Ok(results.into());
    }

    // Read blobs with a given prefix, returning alternating strings of the suffix and value.
    pub fn read_with_prefix(&self, ctx: &Context, args: Vec<String>) -> RedisResult {
        let mut args = args.clone();
        if args.len() != 3 {
            return Err(RedisError::WrongArity);
        }
        let remainder = &args.split_off(2);
        let mut chunks = args.chunks(2);
        // First chunk is command and repo ID
        if let Some([command, repo_uuid]) = chunks.next() {
            match self.get_storage_for_repo(repo_uuid) {
                Ok(storage) => {
                    // Get the key
                    if let Some(key) = remainder.first() {
                        let normalized_command = command.to_ascii_uppercase();
                        ctx.log_notice(
                            format!("Processing command {}('{}')'", &normalized_command, &key)
                                .as_str(),
                        );

                        match storage.get_by_prefix(key.as_bytes()) {
                            Ok(dict) => {
                                let mut outer = Vec::<RedisValue>::new();
                                for (key, value) in dict {
                                    unsafe {
                                        outer.push(RedisValue::BulkString(String::from_utf8_unchecked(key)));
                                        outer.push(RedisValue::BulkString(String::from_utf8_unchecked(value)));
                                    }
                                }
                                Ok(RedisValue::Array(outer))
                            },
                            Err(e) => {
                                let diag = format!("Get('{}') failed: {}", key, e);
                                ctx.log_warning(&diag);
                                Err(RedisError::String(diag))
                            }
                        }
                    } else {
                        unreachable!("Illegal arity should have been caught");
                    }
                }
                Err(e) => {
                    ctx.log_warning("Returning error to client");
                    return Err(RedisError::String(e.into()));
                }
            }
        } else {
            unreachable!("Illegal arity should have been caught");
        }

    }

    pub fn read_blob(&self, ctx: &Context, args: Vec<String>) -> RedisResult {
        let mut args = args.clone();
        if args.len() != 3 {
            return Err(RedisError::WrongArity);
        }
        let remainder = &args.split_off(2);
        let mut chunks = args.chunks(2);
        // First chunk is command and repo ID
        if let Some([command, repo_uuid]) = chunks.next() {
            match self.get_storage_for_repo(repo_uuid) {
                Ok(storage) => {
                    // let storage = storage.clone();
                    // Subsequent chunk is [key, value]
                    if let Some(key) = remainder.first() {
                        let normalized_command = command.to_ascii_uppercase();
                        // ctx.log_notice(
                        //     format!("Processing command {}('{}')'", &normalized_command, &key)
                        //         .as_str(),
                        // );
                        // let mut store = storage.get_bytes(key.as_bytes());
                        match storage.get_bytes(key.as_bytes()) {
                            // redis
                            Ok(Some(bytes)) => {
                                unsafe {
                                    let byte_string = String::from_utf8_unchecked(bytes.into());
                                    return Ok(RedisValue::BulkString(byte_string))
                                }
                            },
                            Ok(None) => return Ok(RedisValue::Null),
                            Err(e) => {
                                let diag = format!("Get('{}') failed: {}", key, e);
                                ctx.log_warning(&diag);
                                return Err(RedisError::String(diag));
                            }
                        }
                    } else {
                        unreachable!("Illegal arity should have been caught");
                    }
                }
                Err(e) => {
                    ctx.log_warning("Returning error to client");
                    return Err(RedisError::String(e.into()));
                }
            }
        } else {
            unreachable!("Illegal arity should have been caught");
        }
    }

    pub fn write_blob(&self, ctx: &Context, args: Vec<String>) -> RedisResult {
        let mut args = args.clone();

        if args.len() != 4 {
            return Err(RedisError::WrongArity);
        }

        debug!("write: enumerating args:");
        for arg in &args {
            debug!("{}", pretty_hex::pretty_hex(&arg));
        }

        let mut chunks = args.chunks(2);
        // First chunk is command and repo ID
        if let Some([command, repo_uuid]) = chunks.next() {
            match self.get_storage_for_repo(repo_uuid) {
                Ok(storage) => {
                    // Subsequent chunk is [key, value]
                    if let Some([key, value]) = chunks.next() {
                        match storage.put_bytes(key.as_bytes(), value.as_bytes()) {
                            Ok(_) => return Ok(RedisValue::Null),
                            Err(e) => return Err(RedisError::Str("put failed")),
                        }
                    } else {
                        unreachable!("Illegal arity should have been caught");
                    }
                }
                Err(e) => return Err(RedisError::String(e.into())),
            }
        } else {
            return Err(RedisError::Str("missing first chunk [command, repo_id]"));
        }
    }

    pub fn count_blob(&self, ctx: &Context, args: Vec<String>) -> RedisResult {
        let mut args = args.clone();

        if args.len() != 4 {
            return Err(RedisError::WrongArity);
        }

        let mut chunks = args.chunks(2);
        // First chunk is command and repo ID
        if let Some([command, repo_uuid]) = chunks.next() {
            match self.get_storage_for_repo(repo_uuid) {
                Ok(storage) => {
                    match storage.estimate_num_keys() {
                        Ok(est) => return Ok(RedisValue::Integer(est)),
                        Err(e) => return Err(RedisError::String(e.into())),
                    }
                }
                Err(e) => return Err(RedisError::String(e.into())),
            }
        } else {
            return Err(RedisError::WrongArity);
        }
    }

    pub fn shutdown(&self, ctx: &Context, args: Vec<String>) -> RedisResult {
        if let Err(e) = self.repos.shutdown() {
            return Err(RedisError::String(e.into()));
        }
        ctx.log_notice("Clean shutdown complete");
        std::process::exit(0);
    }
}
