// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashSet;

use focus_platform::phabricator::{
    differential_changeset_search, differential_query, query, user_whoami,
};

fn main() {
    let response = query::<user_whoami::Endpoint>(user_whoami::Request {}).unwrap();
    println!("Hello, {}!", response.realName);

    let response = query::<differential_query::Endpoint>(differential_query::Request {
        authors: Some(vec![response.phid]),
        limit: Some(10),
        ..Default::default()
    })
    .unwrap();

    println!(
        "Here are some revisions you recently created ({}):",
        response.0.len()
    );
    for revision in &response.0 {
        println!("\t{} {}", revision.uri, revision.title);
    }

    let paths =
        query::<differential_changeset_search::Endpoint>(differential_changeset_search::Request {
            constraints: differential_changeset_search::Constraints {
                diffPHIDs: Some(
                    response
                        .0
                        .iter()
                        .filter_map(|x| x.activeDiffPHID.clone())
                        .collect(),
                ),
            },
        })
        .unwrap();

    let paths: HashSet<_> = paths
        .data
        .into_iter()
        .map(|item| item.fields.path.displayPath)
        .collect();
    println!(
        "Here are some paths you have recently touched ({}):",
        paths.len()
    );
    for path in paths {
        println!("\t{path}");
    }
}
