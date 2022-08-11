#![allow(non_snake_case)]
// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::{
    io,
    process::{Command, Output, Stdio},
};

use chrono::serde::ts_seconds_option;
use chrono::{DateTime, Utc};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use thiserror::Error;

pub trait Endpoint {
    const METHOD_NAME: &'static str;
    type Request: std::fmt::Debug + Serialize;
    type Response: std::fmt::Debug + DeserializeOwned;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PHID(pub String);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Data<F> {
    pub id: usize,
    pub phid: PHID,
    pub fields: F,
}

pub mod user_whoami {
    pub use super::*;

    #[derive(Clone, Debug, Serialize)]
    pub struct Request {}

    #[derive(Clone, Debug, Deserialize)]
    pub struct Response {
        pub phid: PHID,
        pub userName: String,
        pub realName: String,
        pub image: String,
        pub uri: String,
        pub roles: Vec<String>,
        pub primaryEmail: String,
    }

    pub enum Endpoint {}
    impl super::Endpoint for Endpoint {
        const METHOD_NAME: &'static str = "user.whoami";
        type Request = Request;
        type Response = Response;
    }
}

pub mod differential_revision_search {
    use super::*;

    #[derive(Clone, Debug, Default, Serialize)]
    pub struct Constraints {
        // not exhaustive, see
        // https://secure.phabricator.com/conduit/method/differential.revision.search/
        // for more fields
        #[serde(skip_serializing_if = "Option::is_none")]
        pub authorPHIDs: Option<Vec<PHID>>,

        #[serde(skip_serializing_if = "Option::is_none")]
        pub reviewerPHIDs: Option<Vec<PHID>>,

        #[serde(with = "ts_seconds_option")]
        #[serde(skip_serializing_if = "Option::is_none")]
        pub createdStart: Option<DateTime<Utc>>,

        #[serde(with = "ts_seconds_option")]
        #[serde(skip_serializing_if = "Option::is_none")]
        pub createdEnd: Option<DateTime<Utc>>,
    }

    #[derive(Clone, Debug, Default, Serialize)]
    pub struct Request {
        pub constraints: Constraints,
    }

    #[derive(Clone, Debug, Deserialize)]
    pub struct Item {
        pub title: String,
        pub uri: String,
    }

    #[derive(Clone, Debug, Deserialize)]
    pub struct Response {
        // not exhaustive, see
        // https://secure.phabricator.com/conduit/method/differential.revision.search/
        // for more fields
        pub data: Vec<Data<Item>>,
    }

    pub enum Endpoint {}
    impl super::Endpoint for Endpoint {
        const METHOD_NAME: &'static str = "differential.revision.search";
        type Request = Request;
        type Response = Response;
    }
}

pub mod differential_query {
    use super::*;

    #[derive(Clone, Debug, Default, Serialize)]
    pub struct Request {
        // not exhaustive, see
        // https://secure.phabricator.com/conduit/method/differential.query/ for
        // more fields
        #[serde(skip_serializing_if = "Option::is_none")]
        pub authors: Option<Vec<PHID>>,

        #[serde(skip_serializing_if = "Option::is_none")]
        pub status: Option<String>,

        #[serde(skip_serializing_if = "Option::is_none")]
        pub limit: Option<usize>,
    }

    #[derive(Clone, Debug, Deserialize)]
    pub struct Item {
        pub id: String,
        pub phid: PHID,
        pub title: String,
        pub uri: String,
        pub statusName: Option<String>,

        #[serde(default)]
        pub hashes: Vec<(String, String)>,

        pub activeDiffPHID: Option<PHID>,
    }

    #[derive(Clone, Debug, Deserialize)]
    pub struct Response(pub Vec<Item>);

    pub enum Endpoint {}
    impl super::Endpoint for Endpoint {
        const METHOD_NAME: &'static str = "differential.query";
        type Request = Request;
        type Response = Response;
    }
}

pub mod differential_getcommitpaths {
    use super::*;

    #[derive(Clone, Debug, Serialize)]
    pub struct Request {
        pub revision_id: String,
    }

    #[derive(Clone, Debug, Deserialize)]
    pub struct Response(pub Vec<String>);

    pub enum Endpoint {}
    impl super::Endpoint for Endpoint {
        const METHOD_NAME: &'static str = "differential.getcommitpaths";
        type Request = Request;
        type Response = Response;
    }
}

pub mod differential_changeset_search {
    use super::*;

    #[derive(Clone, Debug, Serialize)]
    pub struct Constraints {
        pub diffPHIDs: Option<Vec<PHID>>,
    }

    #[derive(Clone, Debug, Serialize)]
    pub struct Request {
        // not exhaustive, see
        // https://secure.phabricator.com/conduit/method/differential.changeset.search/
        // for more fields
        pub constraints: Constraints,
    }

    #[derive(Clone, Debug, Deserialize)]
    pub struct Path {
        pub displayPath: String,
    }

    #[derive(Clone, Debug, Deserialize)]
    pub struct Item {
        pub diffPHID: PHID,
        pub path: Path,
    }

    #[derive(Clone, Debug, Deserialize)]
    pub struct Response {
        pub data: Vec<Data<Item>>,
    }

    pub enum Endpoint {}
    impl super::Endpoint for Endpoint {
        const METHOD_NAME: &'static str = "differential.changeset.search";
        type Request = Request;
        type Response = Response;
    }
}

#[derive(Clone, Debug, Deserialize)]
struct ResponseWrapper<T> {
    error: Option<String>,
    errorMessage: Option<String>,
    response: Option<T>,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to spawn arc: {0}")]
    Spawn(#[source] io::Error),

    #[error("failed to write stdin to arc: {0}")]
    WriteStdin(#[source] serde_json::Error),

    #[error("failed to read stdout from arc: {0}")]
    ReadStdout(#[source] io::Error),

    #[error("command failed with exit code {output:?}")]
    CommandFailed { output: Output },

    #[error("API call failed ({error}): {message:?}: for value: {output:?}")]
    Api {
        error: String,
        message: Option<String>,
        output: Output,
    },

    #[error("failed to deserialize output with error: {source} for value: {output:?}")]
    Deserialize {
        source: serde_json::Error,
        output: Output,
    },
}

pub fn query<E: Endpoint>(request: E::Request) -> Result<E::Response, Error> {
    let mut child = Command::new("arc")
        .arg("call-conduit")
        .arg("--")
        .arg(E::METHOD_NAME)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(Error::Spawn)?;

    let stdin = child.stdin.take().unwrap();
    serde_json::to_writer(stdin, &request).map_err(Error::WriteStdin)?;
    let output = child.wait_with_output().map_err(Error::ReadStdout)?;
    if !output.status.success() {
        return Err(Error::CommandFailed { output });
    }

    let response: ResponseWrapper<E::Response> = serde_json::from_reader(output.stdout.as_slice())
        .map_err(|source| Error::Deserialize {
            source,
            output: output.clone(),
        })?;
    match response {
        ResponseWrapper {
            error: Some(error),
            errorMessage,
            response: _,
        } => Err(Error::Api {
            error,
            message: errorMessage,
            output,
        }),

        ResponseWrapper {
            error: None,
            errorMessage,
            response: None,
        } => Err(Error::Api {
            error: "<no response>".to_string(),
            message: errorMessage,
            output,
        }),

        ResponseWrapper {
            error: _,
            errorMessage: _,
            response: Some(response),
        } => Ok(response),
    }
}
