//! This crate is [Tool Insights] bindings for Rust.
//! It does not implement all the functionality and configuration as advertised by [Tool Insights].
//! The user can change the log message write location by setting `TOOL_INSIGHTS_DIR`,
//! the default location is `/opt/twitter_mde/var/log/toolinsights`.
//!
//! Init the client at the start of execution and write the log at the end of execution:
//! ```
//! use std::collections::HashMap;
//! use std::time::SystemTime;
//! use tool_insights_client::Client;
//!
//! let ti_client = Client::new(
//!     "tool_name".to_string(),
//!     "tool_version".to_string(),
//!     SystemTime::now(),
//! );
//! // Optionally, add feature name.
//! ti_client.get_context().set_tool_feature_name("example");
//!
//! // do work ...
//! // ...
//!
//! // Optionally, save any other info about the run.
//! ti_client.get_context().add_to_custom_map("current_gas_price", "5.4");
//! // write message at the end of execution.
//! ti_client.get_inner().write_invocation_message(
//!     Some(0), // exit_code
//!     None, // Optionally, this can be a HashMap<String, String> with any data about the run
//! );
//! ```
//!
//! TODO: Troubleshoot why `Drop` for Client does not run reliably.
//! [Tool Insights]: https://docbird.twitter.biz/tool_insights/instrumenting.html

use std::borrow::{Borrow, BorrowMut};
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::SystemTime;

use anyhow::Result;
use tracing::error;

use crate::json_writer::JsonWriter;
use crate::message::{Message, MessageKind};
use crate::writer::Writer;

#[derive(Clone)]
/// Contains metadata about the tool as well as methods to create and write Tool insights logs.
pub struct Client {
    // `inner` here is to abstract away the `.lock().unwrap()` from the user.
    inner: Arc<Mutex<Underlying>>,
}

impl Client {
    pub fn new(tool_name: String, tool_version: String, start_time: SystemTime) -> Client {
        let inner = Arc::from(Mutex::new(Underlying::new(
            tool_name,
            tool_version,
            start_time,
        )));
        Client { inner }
    }

    pub fn get_inner(&self) -> MutexGuard<'_, Underlying> {
        // TODO: Might make sense to have a get_inner() and get_inner_mut here if/when we are using a RwLock instead ofg a Mutex.
        self.inner.lock().unwrap()
    }

    pub fn get_context(&self) -> ClientGuard {
        ClientGuard {
            guard: self.inner.lock().unwrap(),
        }
    }
}

/// This exists so we can give library users a more succinct
/// way to access the `ToolInsightsContext` instance in `ToolInsightsClientInner`.
/// With this guard, the user would access the `ToolInsightsContext` using
/// `$ti_client.get_ti_context()`. Without the guard, the user would need to use
/// `$ti_client.inner().get_ti_context()` or `$ti_client.inner().get_ti_context_mut()`
pub struct ClientGuard<'a> {
    guard: MutexGuard<'a, Underlying>,
}

impl Deref for ClientGuard<'_> {
    type Target = Context;

    fn deref(&self) -> &Self::Target {
        self.guard.get_ti_context()
    }
}

impl DerefMut for ClientGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.guard.ti_context.borrow_mut()
    }
}

pub struct Underlying {
    messages: Vec<Message>,
    ti_context: Context,
    writer: JsonWriter,
}

impl Underlying {
    pub fn new(tool_name: String, tool_version: String, start_time: SystemTime) -> Underlying {
        let ti_context = Context::new(tool_name, tool_version, start_time);
        Underlying {
            messages: vec![],
            ti_context,
            writer: JsonWriter::new(None),
        }
    }

    pub fn get_ti_context(&self) -> &Context {
        self.ti_context.borrow()
    }

    pub fn get_ti_context_mut(&mut self) -> &mut Context {
        self.ti_context.borrow_mut()
    }

    pub fn write_message(&self) -> Result<()> {
        if !self.messages.is_empty() {
            self.writer.write(&self.messages)
        } else {
            Ok(())
        }
    }

    pub fn add_invocation_message(
        &mut self,
        end_time: SystemTime,
        exit_code: Option<i32>,
        custom_map: Option<&HashMap<String, String>>,
    ) {
        if let Some(val) = exit_code {
            if self.get_ti_context().get_exit_code().is_none() {
                self.get_ti_context_mut().set_exit_code(val);
            }
        }
        let message = Message::new(
            MessageKind::PerformanceMessage,
            self.get_ti_context(),
            Some(end_time),
            custom_map,
        );
        self.add_message(message);
    }

    pub fn write_invocation_message(
        &mut self,
        exit_code: Option<i32>,
        custom_map: Option<&HashMap<String, String>>,
    ) {
        self.add_invocation_message(SystemTime::now(), exit_code, custom_map);
        if self.write_message().is_err() {
            error!("Could not write TI message");
        }
    }

    fn add_message(&mut self, message: Message) {
        self.messages.push(message);
    }
}

// Disabling the `Drop` for now because it does not run reliably. The burden is on the user to write
// the TI message at the end, for now.
// impl Drop for ToolInsightsClient {
//     fn drop(&mut self) {
//         self.get_inner()
//             .add_invocation_message(SystemTime::now(), None, None);
//         if let Err(e) = self.get_inner().write_message() {
//             log::info!("Could not write TI message: {}", e);
//         }
//     }
// }

pub struct Context {
    tool_name: String,
    tool_version: String,
    tool_feature_name: Option<String>,
    start_time: SystemTime,
    custom_map: Option<HashMap<String, String>>,
    exit_code: Option<i32>,
}

impl Context {
    fn new(tool_name: String, tool_version: String, start_time: SystemTime) -> Context {
        Context {
            tool_name,
            tool_version,
            tool_feature_name: None,
            start_time,
            custom_map: None,
            exit_code: None,
        }
    }

    pub fn set_tool_feature_name(&mut self, tool_feature_name: impl Into<String>) {
        self.tool_feature_name = Some(tool_feature_name.into());
    }
    pub fn set_custom_map(&mut self, custom_map: HashMap<String, String>) {
        self.custom_map = Some(custom_map);
    }
    pub fn set_exit_code(&mut self, exit_code: i32) {
        self.exit_code = Some(exit_code);
    }

    pub fn add_to_custom_map(&mut self, key: impl Into<String>, value: impl Into<String>) {
        let key = key.into();
        let value = value.into();
        match self.custom_map {
            Some(ref mut map) => map.insert(key, value),
            None => {
                self.custom_map = Some(HashMap::new());
                self.custom_map.as_mut().unwrap().insert(key, value)
            }
        };
    }

    pub fn get_tool_name(&self) -> &str {
        self.tool_name.as_ref()
    }

    pub fn get_tool_version(&self) -> &str {
        self.tool_version.as_ref()
    }

    pub fn get_tool_feature_name(&self) -> Option<&str> {
        self.tool_feature_name.as_deref()
    }

    pub fn get_custom_map(&self) -> Option<&HashMap<String, String>> {
        self.custom_map.as_ref()
    }

    pub fn get_exit_code(&self) -> Option<i32> {
        self.exit_code
    }

    pub fn get_start_time(&self) -> SystemTime {
        self.start_time
    }
}
