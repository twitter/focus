use std::collections::HashMap;
use std::env;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::client::Context;
use crate::util::{
    decode_zipkin_compatible_id, duration_in_seconds, encode_zipkin_compatible_id, get_cwd,
    get_zipkin_compatible_id, merge_maps, seconds_since_time,
};

const SCHEMA_VERSION: u32 = 1;
const TOOL_INSIGHTS_NESTING_LEVEL_ENV_VAR: &str = "TOOL_INSIGHTS_NESTING_LEVEL";
const TOOL_INSIGHTS_SESSION_ID_ENV_VAR: &str = "TOOL_INSIGHTS_SESSION_ID";
const GITSTATS_TRACE_ID: &str = "X_B3_TRACEID";
const GITSTATS_SPAN_ID: &str = "X_B3_SPANID";

// Even though there is a provision to have more than one message here, I'm only
// seeing a single message for every record so that's how we will use it.
#[derive(Serialize, Debug)]
pub struct Message {
    schema_version: u32,
    messages: Vec<MessageBody>,
}

impl Message {
    pub fn new(
        message_type: MessageKind,
        ti_context: &Context,
        end_time: Option<SystemTime>,
        map: Option<&HashMap<String, String>>,
    ) -> Message {
        let message = MessageBody::new(message_type, ti_context, end_time, map);
        let messages = vec![message];
        Message {
            schema_version: SCHEMA_VERSION,
            messages,
        }
    }
    #[allow(dead_code)]
    fn add_message(&mut self, message: MessageBody) -> &Message {
        self.messages.push(message);
        self
    }
}

#[derive(Serialize, Debug)]
pub struct MessageBody {
    message_type: String,
    core_data: CoreData,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_seconds: Option<f64>,
}

impl MessageBody {
    pub fn new(
        message_type: MessageKind,
        ti_context: &Context,
        end_time: Option<SystemTime>,
        map: Option<&HashMap<String, String>>,
    ) -> MessageBody {
        MessageBody {
            message_type: message_type.to_string(),
            core_data: CoreData::new(ti_context, map),
            duration_seconds: end_time.map(|t| duration_in_seconds(ti_context.get_start_time(), t)),
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct CoreData {
    tool_name: String,
    tool_version: String,
    tool_feature_name: String,
    run_id: String,
    run_time_epoch: u64,
    run_nesting_level: u32,
    run_argv: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    run_exit_code: Option<i32>,
    run_current_working_directory: String,
    session_id: String,
    user_username: String,
    machine_hostname: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    custom_map: Option<HashMap<String, String>>,
}

impl CoreData {
    fn new(ti_context: &Context, map: Option<&HashMap<String, String>>) -> CoreData {
        let final_map: Option<HashMap<String, String>> =
            merge_maps(map.cloned(), ti_context.get_custom_map().cloned());

        let core_data = CoreData {
            tool_name: ti_context.get_tool_name().to_string(),
            tool_version: ti_context.get_tool_version().to_string(),
            tool_feature_name: ti_context
                .get_tool_feature_name()
                .unwrap_or("__invocation__")
                .to_string(),
            run_id: Uuid::new_v4().to_string(),
            run_time_epoch: seconds_since_time(ti_context.get_start_time()),
            run_nesting_level: get_nesting_level(),
            run_argv: env::args().collect(),
            run_exit_code: ti_context.get_exit_code(),
            run_current_working_directory: get_cwd()
                .into_os_string()
                .into_string()
                .unwrap_or_else(|_| "no_cwd".to_string()),
            session_id: get_session_id(),
            user_username: whoami::username(),
            machine_hostname: whoami::hostname(),
            custom_map: final_map,
        };
        set_env_vars(&core_data);
        core_data
    }
}

pub enum MessageKind {
    ErrorMessage,
    PerformanceMessage,
    UsageMessage,
}

impl ToString for MessageKind {
    fn to_string(&self) -> String {
        match *self {
            MessageKind::ErrorMessage => {
                "com.twitter.toolinsights.messages.ErrorMessage".to_string()
            }
            MessageKind::PerformanceMessage => {
                "com.twitter.toolinsights.messages.PerformanceMessage".to_string()
            }
            MessageKind::UsageMessage => {
                "com.twitter.toolinsights.messages.UsageMessage".to_string()
            }
        }
    }
}

pub fn get_session_id() -> String {
    // return value from environment variable, if set, otherwise a new uuid
    env::var(TOOL_INSIGHTS_SESSION_ID_ENV_VAR).unwrap_or_else(|_| Uuid::new_v4().to_string())
}

pub fn get_trace_id() -> u64 {
    // If the trace id env var is set, use that value, otherwise generate new
    match env::var(GITSTATS_TRACE_ID) {
        Ok(id_str) => decode_zipkin_compatible_id(id_str),
        Err(_) => get_zipkin_compatible_id(),
    }
}

pub fn get_span_id(trace_id: u64) -> u64 {
    // If the span id env var is set, then generate a new span id, otherwise use trace id
    match env::var(GITSTATS_SPAN_ID) {
        Ok(_) => {
            // span id is set, create a new span id for this process
            get_zipkin_compatible_id()
        }
        Err(_) => {
            //span id is not set, use trace id
            trace_id
        }
    }
}

pub fn set_trace_env_vars(trace_id: u64, span_id: u64) {
    env::set_var(
        GITSTATS_TRACE_ID,
        encode_zipkin_compatible_id(trace_id, false),
    );
    env::set_var(
        GITSTATS_SPAN_ID,
        encode_zipkin_compatible_id(span_id, false),
    );
}

pub fn get_nesting_level() -> u32 {
    // return value from environment variable (if set), otherwise `0`
    env::var(TOOL_INSIGHTS_NESTING_LEVEL_ENV_VAR)
        .unwrap_or_else(|_| "0".to_string())
        .parse()
        .unwrap_or(0)
}

pub fn set_env_vars(data: &CoreData) {
    env::set_var(TOOL_INSIGHTS_SESSION_ID_ENV_VAR, &data.session_id);
    env::set_var(
        TOOL_INSIGHTS_NESTING_LEVEL_ENV_VAR,
        (data.run_nesting_level + 1).to_string(),
    );
}
