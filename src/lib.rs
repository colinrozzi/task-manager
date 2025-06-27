#[allow(warnings)]
mod bindings;
mod protocol;

use bindings::exports::theater::simple::actor::Guest;
use bindings::exports::theater::simple::message_server_client::Guest as MessageServerClient;
use bindings::exports::theater::simple::supervisor_handlers::Guest as SupervisorHandlers;
use bindings::theater::simple::message_server_host::send;
use bindings::theater::simple::runtime::{log, shutdown};
use bindings::theater::simple::supervisor::spawn;
use bindings::theater::simple::types::{ChannelAccept, ChannelId, WitActorError};
use genai_types::{Message, MessageContent, messages::Role};
use serde::{Deserialize, Serialize};
use serde_json::{from_slice, to_vec, Value};

struct Component;

const CHAT_STATE_MANIFEST_PATH: &str =
    "/Users/colinrozzi/work/actor-registry/chat-state/manifest.toml";
const DEFAULT_TASK_MONITOR_MANIFEST_PATH: &str =
    "https://github.com/colinrozzi/task-monitor-mcp-actor/releases/latest/download/manifest.toml";

// Protocol types for external communication
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
enum TaskManagerRequest {
    GetChatStateActorId,
    AddMessage { message: Message },
    StartChat,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
enum TaskManagerResponse {
    ChatStateActorId { actor_id: String },
    Success,
    Error { message: String },
}

// Configuration for task manager
#[derive(Serialize, Deserialize, Debug)]
struct TaskManagerConfig {
    // Core task definition
    system_prompt: Option<String>,
    initial_message: Option<String>,
    
    // AI configuration  
    model_config: Option<Value>,
    temperature: Option<f64>,
    max_tokens: Option<u32>,
    
    // Tool configuration
    mcp_servers: Option<Value>,
    
    // Execution mode
    auto_exit_on_completion: Option<bool>,
    
    #[serde(flatten)]
    other: Value,
}

impl Default for TaskManagerConfig {
    fn default() -> Self {
        Self {
            system_prompt: None,
            initial_message: None,
            model_config: None,
            temperature: None,
            max_tokens: None,
            mcp_servers: None,
            auto_exit_on_completion: None,
            other: serde_json::json!({}),
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct TaskComplete;

// State management
#[derive(Serialize, Deserialize, Debug)]
struct TaskManagerState {
    actor_id: String,
    chat_state_actor_id: Option<String>,
    original_config: Value,
    initial_message: Option<String>,
}

impl TaskManagerState {
    fn new(
        actor_id: String,
        config: Value,
        initial_message: Option<String>,
    ) -> Self {
        Self {
            actor_id,
            chat_state_actor_id: None,
            original_config: config,
            initial_message,
        }
    }

    fn set_chat_state_actor_id(&mut self, chat_actor_id: String) {
        self.chat_state_actor_id = Some(chat_actor_id);
    }

    fn get_chat_state_actor_id(&self) -> Result<&String, String> {
        self.chat_state_actor_id
            .as_ref()
            .ok_or_else(|| "Chat state actor not initialized".to_string())
    }
}

impl Guest for Component {
    fn init(state: Option<Vec<u8>>, params: (String,)) -> Result<(Option<Vec<u8>>,), String> {
        log("Task manager actor initializing...");

        let (self_id,) = params;

        // Parse initial configuration if provided
        let (task_config, initial_message) = if let Some(state_bytes) = state {
            match from_slice::<TaskManagerConfig>(&state_bytes) {
                Ok(config) => {
                    log("Parsed initial configuration");
                    let task_config = create_task_config(&self_id, &config);
                    (task_config, config.initial_message)
                }
                Err(e) => {
                    log(&format!(
                        "Failed to parse initial config, using defaults: {}",
                        e
                    ));
                    let task_config = create_task_config(&self_id, &TaskManagerConfig::default());
                    (task_config, None)
                }
            }
        } else {
            log("No initial state provided, using default configuration");
            let task_config = create_task_config(&self_id, &TaskManagerConfig::default());
            (task_config, None)
        };

        log(&format!("Using task config: {}", task_config));

        // Create our state
        let mut task_state = TaskManagerState::new(self_id, task_config.clone(), initial_message);

        // Spawn the chat-state actor with the task config
        match spawn_chat_state_actor(&task_config) {
            Ok(chat_actor_id) => {
                log(&format!("Chat state actor spawned: {}", chat_actor_id));
                task_state.set_chat_state_actor_id(chat_actor_id);
            }
            Err(e) => {
                let error_msg = format!("Failed to spawn chat state actor: {}", e);
                log(&error_msg);
                return Err(error_msg);
            }
        }

        // Serialize our state
        let state_bytes = to_vec(&task_state)
            .map_err(|e| format!("Failed to serialize task state: {}", e))?;

        log("Task manager actor initialized successfully");
        Ok((Some(state_bytes),))
    }
}

impl SupervisorHandlers for Component {
    fn handle_child_error(
        state: Option<Vec<u8>>,
        params: (String, WitActorError),
    ) -> Result<(Option<Vec<u8>>,), String> {
        let (_child_id, _error) = params;
        log("Task manager: Child actor error occurred");
        Ok((state,))
    }

    fn handle_child_exit(
        state: Option<Vec<u8>>,
        params: (String, Option<Vec<u8>>),
    ) -> Result<(Option<Vec<u8>>,), String> {
        let (child_id, _exit_data) = params;
        log(&format!("Task manager: Child actor exited: {}", child_id));

        let task_state: TaskManagerState = match state {
            Some(state_bytes) => match from_slice(&state_bytes) {
                Ok(state) => state,
                Err(e) => {
                    log(&format!("Failed to deserialize task state: {}", e));
                    return Ok((None,));
                }
            },
            None => {
                log("No state available for child exit handler");
                return Ok((None,));
            }
        };

        // If our chat state actor exited, we should probably shut down too
        if let Ok(chat_actor_id) = task_state.get_chat_state_actor_id() {
            if chat_actor_id == &child_id {
                log("Chat state actor exited, shutting down task manager");
                let _ = shutdown(None);
            }
        }

        let updated_state_bytes = to_vec(&task_state).unwrap_or_default();
        Ok((Some(updated_state_bytes),))
    }

    fn handle_child_external_stop(
        state: Option<Vec<u8>>,
        params: (String,),
    ) -> Result<(Option<Vec<u8>>,), String> {
        let (child_id,) = params;
        log(&format!("Task manager: Child actor externally stopped: {}", child_id));
        Ok((state,))
    }
}

impl MessageServerClient for Component {
    fn handle_send(
        state: Option<Vec<u8>>,
        params: (Vec<u8>,),
    ) -> Result<(Option<Vec<u8>>,), String> {
        let (data,) = params;
        log("Task manager handling send message");

        let parsed_state: TaskManagerState = match state {
            Some(state_bytes) => match from_slice(&state_bytes) {
                Ok(state) => state,
                Err(e) => {
                    let error_msg = format!("Failed to deserialize task state: {}", e);
                    log(&error_msg);
                    return Err(error_msg);
                }
            },
            None => {
                let error_msg = "No state available for send_message".to_string();
                log(&error_msg);
                return Err(error_msg);
            }
        };

        // Forward the message to the chat state actor
        match parsed_state.get_chat_state_actor_id() {
            Ok(chat_actor_id) => {
                match send(chat_actor_id, &data) {
                    Ok(_) => {
                        log("Message forwarded to chat state actor");
                    }
                    Err(e) => {
                        let error_msg = format!("Failed to forward message: {:?}", e);
                        log(&error_msg);
                        return Err(error_msg);
                    }
                }
            }
            Err(e) => {
                let error_msg = format!("Chat state actor not available: {}", e);
                log(&error_msg);
                return Err(error_msg);
            }
        }

        let state_bytes = to_vec(&parsed_state).unwrap_or_default();
        Ok((Some(state_bytes),))
    }

    fn handle_request(
        state: Option<Vec<u8>>,
        params: (String, Vec<u8>),
    ) -> Result<(Option<Vec<u8>>, (Option<Vec<u8>>,)), String> {
        let (_request_id, data) = params;
        log("Task manager handling request message");

        // Deserialize current state
        let task_state: TaskManagerState = match state {
            Some(state_bytes) => match from_slice(&state_bytes) {
                Ok(state) => state,
                Err(e) => {
                    let error_msg = format!("Failed to deserialize task state: {}", e);
                    log(&error_msg);
                    let error_response = TaskManagerResponse::Error { message: error_msg };
                    return Ok((
                        None,
                        (Some(to_vec(&error_response).unwrap_or_default()),),
                    ));
                }
            },
            None => {
                let error_response = TaskManagerResponse::Error {
                    message: "No state available".to_string(),
                };
                return Ok((
                    None,
                    (Some(to_vec(&error_response).unwrap_or_default()),),
                ));
            }
        };

        // Parse the request
        let request: TaskManagerRequest = match from_slice(&data) {
            Ok(req) => {
                log(&format!("Parsed request: {:?}", req));
                req
            }
            Err(e) => {
                let error_msg = format!("Failed to parse request: {}", e);
                log(&error_msg);
                let error_response = TaskManagerResponse::Error { message: error_msg };
                return Ok((
                    Some(to_vec(&task_state).unwrap_or_default()),
                    (Some(to_vec(&error_response).unwrap_or_default()),),
                ));
            }
        };

        // Handle the request
        let response = match request {
            TaskManagerRequest::StartChat => {
                log("Handling StartChat request");

                // Send initial message if configured
                if let Some(initial_msg) = &task_state.initial_message {
                    log("Sending initial message to chat state actor");
                    let message = Message {
                        role: Role::User,
                        content: vec![MessageContent::Text {
                            text: initial_msg.clone(),
                        }],
                    };

                    match task_state.get_chat_state_actor_id() {
                        Ok(chat_actor_id) => {
                            let add_message_request = protocol::ChatStateRequest::AddMessage { message };
                            match to_vec(&add_message_request) {
                                Ok(request_data) => {
                                    match send(chat_actor_id, &request_data) {
                                        Ok(_) => {
                                            log("Initial message sent successfully");
                                        }
                                        Err(e) => {
                                            log(&format!("Failed to send initial message: {:?}", e));
                                        }
                                    }
                                }
                                Err(e) => {
                                    log(&format!("Failed to serialize initial message: {}", e));
                                }
                            }
                        }
                        Err(e) => {
                            log(&format!("Chat state actor not available: {}", e));
                        }
                    }
                }

                TaskManagerResponse::Success
            }
            TaskManagerRequest::GetChatStateActorId => match task_state.get_chat_state_actor_id() {
                Ok(chat_actor_id) => {
                    log(&format!("Returning chat state actor ID: {}", chat_actor_id));
                    TaskManagerResponse::ChatStateActorId {
                        actor_id: chat_actor_id.clone(),
                    }
                }
                Err(e) => TaskManagerResponse::Error { message: e },
            },
            TaskManagerRequest::AddMessage { message } => {
                match task_state.get_chat_state_actor_id() {
                    Ok(chat_actor_id) => {
                        let add_message_request = protocol::ChatStateRequest::AddMessage { message };

                        match to_vec(&add_message_request) {
                            Ok(request_data) => {
                                match send(chat_actor_id, &request_data) {
                                    Ok(_) => {
                                        log("Message forwarded to chat state actor");
                                        TaskManagerResponse::Success
                                    }
                                    Err(e) => {
                                        let error_msg = format!("Failed to forward message: {:?}", e);
                                        log(&error_msg);
                                        TaskManagerResponse::Error { message: error_msg }
                                    }
                                }
                            }
                            Err(e) => {
                                let error_msg = format!("Failed to serialize message: {}", e);
                                log(&error_msg);
                                TaskManagerResponse::Error { message: error_msg }
                            }
                        }
                    }
                    Err(e) => TaskManagerResponse::Error { message: e },
                }
            }
        };

        // Serialize response and return
        let response_data = to_vec(&response)
            .map_err(|e| format!("Failed to serialize response: {}", e))?;
        let updated_state_bytes =
            to_vec(&task_state).map_err(|e| format!("Failed to serialize current state: {}", e))?;

        Ok((Some(updated_state_bytes), (Some(response_data),)))
    }

    fn handle_channel_open(
        state: Option<Vec<u8>>,
        params: (String, Vec<u8>),
    ) -> Result<(Option<Vec<u8>>, (ChannelAccept,)), String> {
        let (_channel_id, _data) = params;
        log("Task manager: Channel open request");
        Ok((state, (ChannelAccept { accepted: true, message: None },)))
    }

    fn handle_channel_close(
        state: Option<Vec<u8>>,
        params: (ChannelId,),
    ) -> Result<(Option<Vec<u8>>,), String> {
        let (channel_id,) = params;
        log(&format!(
            "Task manager: Channel closed: {}",
            channel_id
        ));
        Ok((state,))
    }

    fn handle_channel_message(
        state: Option<Vec<u8>>,
        params: (ChannelId, Vec<u8>),
    ) -> Result<(Option<Vec<u8>>,), String> {
        let (channel_id, _data) = params;
        log(&format!(
            "Task manager: Received channel message on: {}",
            channel_id
        ));
        Ok((state,))
    }
}

// Helper functions
fn create_task_config(self_id: &str, config: &TaskManagerConfig) -> Value {
    log("Creating task configuration...");

    // Default system prompt if none provided
    let default_system_prompt = "You are an AI assistant that helps users complete tasks efficiently. You have access to various tools and can help with a wide range of activities.";

    let system_prompt = config
        .system_prompt
        .as_deref()
        .unwrap_or(default_system_prompt);

    // Add task completion instruction to system prompt
    let completion_instruction = "\n\nIMPORTANT: When you have completed your assigned task, you MUST call the 'task_complete' tool to signal that the work is finished. This allows the system to properly conclude the task session.";

    let final_system_prompt = format!("{}{}", system_prompt, completion_instruction);

    // Default model config
    let default_model_config = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "provider": "anthropic"
    });

    let model_config = config
        .model_config
        .as_ref()
        .unwrap_or(&default_model_config);

    // Default temperature and tokens
    let temperature = config.temperature.unwrap_or(0.7);
    let max_tokens = config.max_tokens.unwrap_or(8192);

    // Default MCP servers (just task monitor)
    let default_mcp_servers = serde_json::json!([
        {
            "actor_id": null,
            "actor": {
                "manifest_path": DEFAULT_TASK_MONITOR_MANIFEST_PATH,
                "init_state": {
                    "management_actor": self_id,
                }
            },
            "tools": null
        }
    ]);

    let mcp_servers = config.mcp_servers.as_ref().unwrap_or(&default_mcp_servers);

    log(&format!("Using model: {:?}", model_config));
    log(&format!("Using temperature: {}", temperature));
    log(&format!("Using max_tokens: {}", max_tokens));

    // Build the final configuration
    let mut final_config = serde_json::json!({
        "model_config": model_config,
        "temperature": temperature,
        "max_tokens": max_tokens,
        "system_prompt": final_system_prompt,
        "mcp_servers": mcp_servers
    });

    // Merge any additional fields from the other config
    if let Some(obj) = final_config.as_object_mut() {
        if let Value::Object(other_map) = &config.other {
            for (key, value) in other_map {
                if !obj.contains_key(key) {
                    obj.insert(key.clone(), value.clone());
                }
            }
        }
    }

    log(&format!("Created final task config: {}", final_config));
    final_config
}

fn spawn_chat_state_actor(config: &Value) -> Result<String, String> {
    log("Spawning chat-state actor...");

    let config_bytes = to_vec(config).map_err(|e| format!("Failed to serialize config: {}", e))?;

    match spawn(CHAT_STATE_MANIFEST_PATH, Some(&config_bytes)) {
        Ok(actor_id) => {
            log(&format!("Successfully spawned chat-state actor: {}", actor_id));
            Ok(actor_id)
        }
        Err(e) => {
            let error_msg = format!("Failed to spawn chat-state actor: {:?}", e);
            log(&error_msg);
            Err(error_msg)
        }
    }
}


bindings::export!(Component with_types_in bindings);