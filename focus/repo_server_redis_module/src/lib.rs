#![feature(try_trait)]

extern crate internals;
extern crate log;
extern crate pretty_hex;

#[macro_use]
extern crate redis_module;
mod processor;

use internals::{error::AppError, repo::*, storage::rocks::Storage};
use log::{debug, info, warn};
use redis_module::{parse_integer, Context, RedisError, RedisResult};
use std::sync::Arc;
use structopt::lazy_static::lazy_static;
use processor::Processor;

lazy_static! {
    static ref PROCESSOR: Processor = { Processor::new(None).expect("Instantiating processor failed") };
}

mod commands {
    use std::borrow::BorrowMut;

    use redis_module::{Context, RedisResult};

    pub fn list_repos(ctx: &Context, args: Vec<String>) -> RedisResult {
        super::PROCESSOR.list_repos(ctx, args)
    }

    pub fn read_blob(ctx: &Context, args: Vec<String>) -> RedisResult {
        super::PROCESSOR.read_blob(ctx, args)
    }

    pub fn read_with_prefix(ctx: &Context, args: Vec<String>) -> RedisResult {
        super::PROCESSOR.read_with_prefix(ctx, args)
    }

    pub fn write_blob(ctx: &Context, args: Vec<String>) -> RedisResult {
        super::PROCESSOR.write_blob(ctx, args)
    }

    pub fn count_blob(ctx: &Context, args: Vec<String>) -> RedisResult {
        super::PROCESSOR.count_blob(ctx, args)
    }

    pub fn shutdown(ctx: &Context, args: Vec<String>) -> RedisResult {
        super::PROCESSOR.shutdown(ctx, args)
    }
}

redis_module! {
    name: "reposerver",
    version: 1,
    data_types: [],
    commands: [
        ["reposerver.list_repos", commands::list_repos, "", 0, 0, 0],
        ["reposerver.write_blob", commands::read_with_prefix, "", 0, 0, 0],
        ["reposerver.read_blob", commands::read_blob, "", 0, 0, 0],
        ["reposerver.write_blob", commands::write_blob, "", 0, 0, 0],
        ["reposerver.count_blob", commands::count_blob, "", 0, 0, 0],

        ["reposerver.shutdown", commands::shutdown, "", 0, 0, 0],
    ],
}

//////////////////////////////////////////////////////

#[cfg(test)]
mod tests {
    use super::*;
    use commands::list_repos;
    use redis_module::RedisValue;

    // fn run_hello_mul(args: &[&str]) -> RedisResult {
    //     commands::list_repos(
    //         &Context::dummy(),
    //         args.iter().map(|v| String::from(*v)).collect(),
    //     )
    // }
}
