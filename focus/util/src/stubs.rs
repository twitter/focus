// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Stubs for Twitter-internal infrastructure.

pub mod tool_insights_client {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, MutexGuard};
    use std::time::SystemTime;

    use anyhow::Result;

    #[derive(Clone)]
    pub struct Client {
        underlying: Arc<Mutex<Underlying>>,
    }

    impl Client {
        pub fn new(_tool_name: String, _tool_version: String, _start_time: SystemTime) -> Self {
            Client {
                underlying: Arc::new(Mutex::new(Underlying)),
            }
        }

        pub fn get_context(&self) -> ClientGuard {
            ClientGuard
        }

        pub fn get_inner(&self) -> MutexGuard<'_, Underlying> {
            self.underlying.lock().unwrap()
        }
    }

    pub struct ClientGuard;

    impl ClientGuard {
        pub fn set_tool_feature_name(&mut self, _tool_feature_name: impl Into<String>) {}
        pub fn set_custom_map(&mut self, _custom_map: HashMap<String, String>) {}
        pub fn set_exit_code(&mut self, _exit_code: i32) {}
        pub fn add_to_custom_map(&mut self, _key: impl Into<String>, _value: impl Into<String>) {}
    }

    pub struct Underlying;

    impl Underlying {
        pub fn write_message(&self) -> Result<()> {
            Ok(())
        }

        pub fn add_invocation_message(
            &mut self,
            _end_time: SystemTime,
            _exit_code: Option<i32>,
            _custom_map: Option<&HashMap<String, String>>,
        ) {
        }

        pub fn write_invocation_message(
            &mut self,
            _exit_code: Option<i32>,
            _custom_map: Option<&HashMap<String, String>>,
        ) {
        }
    }
}
