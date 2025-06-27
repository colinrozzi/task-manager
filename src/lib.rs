#[allow(warnings)]
mod bindings;
mod protocol;

use bindings::exports::theater::simple::actor::Guest;
use bindings::exports::theater::simple::message_server_client::Guest as MessageServerClient;
use bindings::exports::theater::simple::supervisor_handlers::Guest as SupervisorHandlers;
use bindings::theater::simple::message_server_host::send;
use bindings::theater::simple::runtime::{log, shutdown};
use bindings::theater::simple::supervisor::spawn;
use bindings::theater::simple::types::{ChannelAccept, Event, WitActorError, WitErrorType};
use genai_types::Message;
use serde::{Deserialize, Serialize};
use serde_json::{from_slice, to_vec, Value};

struct Component;

const CHAT_STATE_MANIFEST_PATH: &str =
    //"https://github.com/colinrozzi/chat-state/releases/latest/download/manifest.toml";
    "/Users/colinrozzi/work/actor-registry/chat-state/manifest.toml";
const TASK_MONITOR_MANIFEST_PATH: &str =
    "https://github.com/colinrozzi/task-monitor-mcp-actor/releases/latest/download/manifest.toml";
const GIT_MCP_MANIFEST_PATH: &str =
    "https://github.com/colinrozzi/git-mcp-actor/releases/latest/download/manifest.toml";

// Protocol types for external communication
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
enum GitChatRequest {
    GetChatStateActorId,
    AddMessage { message: Message },
    StartChat,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
enum GitChatResponse {
    ChatStateActorId { actor_id: String },
    Success,
    Error { message: String },
}

// Configuration for git assistant
#[derive(Serialize, Deserialize, Debug)]
struct GitAssistantConfig {
    current_directory: Option<String>,
    task: Option<String>,
    model_config: Option<Value>,
    temperature: Option<f64>,
    max_tokens: Option<u32>,
    system_prompt: Option<String>,
    title: Option<String>,
    description: Option<String>,
    mcp_servers: Option<Value>,
    #[serde(flatten)]
    other: Value,
}

impl Default for GitAssistantConfig {
    fn default() -> Self {
        Self {
            current_directory: None,
            task: None,
            model_config: None,
            temperature: None,
            max_tokens: None,
            system_prompt: None,
            title: None,
            description: None,
            mcp_servers: None,
            other: serde_json::json!({}),
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct TaskComplete;

// State management
#[derive(Serialize, Deserialize, Debug)]
struct GitChatState {
    actor_id: String,
    chat_state_actor_id: Option<String>,
    original_config: Value,
    current_directory: Option<String>,
    task: Option<String>,
}

impl GitChatState {
    fn new(
        actor_id: String,
        config: Value,
        current_directory: Option<String>,
        task: Option<String>,
    ) -> Self {
        Self {
            actor_id,
            chat_state_actor_id: None,
            original_config: config,
            current_directory,
            task,
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
        log("Git chat assistant actor initializing...");

        let (self_id,) = params;

        // Parse initial configuration if provided
        let (git_config, current_directory, task) = if let Some(state_bytes) = state {
            match from_slice::<GitAssistantConfig>(&state_bytes) {
                Ok(config) => {
                    log(&format!(
                        "Parsed initial config with current_directory: {:?}, task: {:?}",
                        config.current_directory, config.task
                    ));
                    let git_config = create_git_optimized_config(
                        &self_id,
                        config.current_directory.as_deref(),
                        &config,
                    );
                    (git_config, config.current_directory, config.task)
                }
                Err(e) => {
                    log(&format!(
                        "Failed to parse initial config, using defaults: {}",
                        e
                    ));
                    let git_config =
                        create_git_optimized_config(&self_id, None, &GitAssistantConfig::default());
                    (git_config, None, None)
                }
            }
        } else {
            log("No initial state provided, using default configuration");
            let git_config =
                create_git_optimized_config(&self_id, None, &GitAssistantConfig::default());
            (git_config, None, None)
        };

        log(&format!("Using git config: {}", git_config));

        // Create our state
        let mut git_state = GitChatState::new(self_id, git_config.clone(), current_directory, task);

        // Spawn the chat-state actor with the git config
        match spawn_chat_state_actor(&git_config) {
            Ok(chat_actor_id) => {
                log(&format!("Chat state actor spawned: {}", chat_actor_id));
                git_state.set_chat_state_actor_id(chat_actor_id);
            }
            Err(e) => {
                let error_msg = format!("Failed to spawn chat state actor: {}", e);
                log(&error_msg);
                return Err(error_msg);
            }
        }

        // Serialize our state
        let state_bytes =
            to_vec(&git_state).map_err(|e| format!("Failed to serialize git state: {}", e))?;

        log("Git chat assistant actor initialized successfully");
        Ok((Some(state_bytes),))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash)]
pub struct ChainEvent {
    /// Cryptographic hash of this event's content, used as its identifier.
    /// This is calculated based on all other fields except the hash itself.
    pub hash: Vec<u8>,
    /// Hash of the parent event, or None if this is the first event in the chain.
    /// This creates the cryptographic linking between events.
    pub parent_hash: Option<Vec<u8>>,
    /// Type identifier for the event, used to categorize and filter events.
    /// Common types include "state_change", "message", "http_request", etc.
    pub event_type: String,
    /// The actual payload of the event, typically serialized structured data.
    pub data: Vec<u8>,
    /// Unix timestamp (in seconds) when the event was created.
    pub timestamp: u64,
    /// Optional human-readable description of the event for logging and debugging.
    pub description: Option<String>,
}

impl SupervisorHandlers for Component {
    fn handle_child_error(
        state: Option<Vec<u8>>,
        params: (String, WitActorError),
    ) -> Result<(Option<Vec<u8>>,), String> {
        let (child, error) = params;

        log(&format!(
            "Child {} encountered an error: {:?}",
            child, error
        ));

        match error {
            WitActorError {
                error_type: WitErrorType::Internal,
                data,
            } => {
                log("Internal error type");
                let error_event: ChainEvent = match from_slice(&data.unwrap()) {
                    Ok(event) => event,
                    Err(e) => {
                        let error_msg = format!("Failed to parse internal error data: {}", e);
                        log(&error_msg);
                        return Err(error_msg);
                    }
                };

                log(&format!("Internal error event: {:?}", error_event));

                let error_str = String::from_utf8_lossy(&error_event.data);
                Err(format!("Internal error in child {}: {}", child, error_str))
            }
            _ => {
                log("Other error type");
                let data = error.data.unwrap();
                log(&format!("Error data: {:?}", data));
                let error_str = String::from_utf8_lossy(&data);
                Err(format!("Other error in child {}: {}", child, error_str))
            }
        }
    }

    fn handle_child_exit(
        state: Option<Vec<u8>>,
        params: (String, Option<Vec<u8>>),
    ) -> Result<(Option<Vec<u8>>,), String> {
        let (child_id, _exit_state) = params;
        log(&format!("Child exit: {}", child_id));
        Ok((state,))
    }

    fn handle_child_external_stop(
        state: Option<Vec<u8>>,
        params: (String,),
    ) -> Result<(Option<Vec<u8>>,), String> {
        let (child_id,) = params;
        log(&format!("Child external stop: {}", child_id));
        Ok((state,))
    }
}

impl MessageServerClient for Component {
    fn handle_send(
        state: Option<Vec<u8>>,
        params: (Vec<u8>,),
    ) -> Result<(Option<Vec<u8>>,), String> {
        log("Git chat assistant handling send message");

        let parsed_state: GitChatState = match state {
            Some(state_bytes) => match from_slice(&state_bytes) {
                Ok(state) => state,
                Err(e) => {
                    let error_msg = format!("Failed to deserialize git state: {}", e);
                    log(&error_msg);
                    return Err(error_msg);
                }
            },
            None => {
                let error_msg = "No state available";
                log(error_msg);
                return Err(error_msg.to_string());
            }
        };

        match from_slice::<TaskComplete>(&params.0) {
            Ok(msg) => {
                log(&format!("Received task completion message: {:?}", msg));

                let _ = shutdown(None);
            }
            Err(e) => {
                let error_msg = format!("Failed to parse message: {}", e);
                log(&error_msg);
                return Err(error_msg);
            }
        };

        let updated_state = to_vec(&parsed_state)
            .map_err(|e| format!("Failed to serialize updated state: {}", e))?;
        Ok((Some(updated_state),))
    }

    fn handle_request(
        state: Option<Vec<u8>>,
        params: (String, Vec<u8>),
    ) -> Result<(Option<Vec<u8>>, (Option<Vec<u8>>,)), String> {
        log("Git chat assistant handling request message");

        let (_request_id, data) = params;

        // Deserialize our state
        let git_state: GitChatState = match state {
            Some(state_bytes) => match from_slice(&state_bytes) {
                Ok(state) => state,
                Err(e) => {
                    let error_msg = format!("Failed to deserialize git state: {}", e);
                    log(&error_msg);
                    let error_response = GitChatResponse::Error { message: error_msg };
                    let response_bytes = to_vec(&error_response)
                        .map_err(|e| format!("Failed to serialize error response: {}", e))?;
                    return Ok((None, (Some(response_bytes),)));
                }
            },
            None => {
                let error_msg = "No state available";
                log(error_msg);
                let error_response = GitChatResponse::Error {
                    message: error_msg.to_string(),
                };
                let response_bytes = to_vec(&error_response)
                    .map_err(|e| format!("Failed to serialize error response: {}", e))?;
                return Ok((None, (Some(response_bytes),)));
            }
        };

        // Parse the request
        let request: GitChatRequest = match from_slice(&data) {
            Ok(req) => {
                log(&format!("Parsed request: {:?}", req));
                req
            }
            Err(e) => {
                let error_msg = format!("Failed to parse request: {}", e);
                log(&error_msg);
                let error_response = GitChatResponse::Error { message: error_msg };
                let response_bytes = to_vec(&error_response)
                    .map_err(|e| format!("Failed to serialize error response: {}", e))?;
                return Ok((
                    Some(to_vec(&git_state).unwrap_or_default()),
                    (Some(response_bytes),),
                ));
            }
        };

        // Handle the request
        let response = match request {
            GitChatRequest::StartChat => {
                log("Starting task session...");

                // Check if we have a task that requires auto-initiation
                if let Some(task) = &git_state.task {
                    log(&format!("Auto-initiating task: {}", task));

                    let auto_message = match task.as_str() {
                        "commit" => "Please analyze the repository and commit any pending changes with appropriate commit messages. Start by checking git status to see what files have changed.",
                        "review" => "Please perform a comprehensive code review of the current changes. Start by examining what has been modified.",
                        "rebase" => "Please help me clean up the git history through an interactive rebase. Start by showing the current commit history.",
                        "analyze" => "Please provide a comprehensive analysis of this repository. Start by examining the overall structure and recent activity.",
                        "cleanup" => "Please help clean up and organize this repository. Start by identifying what needs attention.",
                        _ => "Please proceed with the assigned task. Let me know if you need clarification on what should be done.",
                    };

                    match git_state.get_chat_state_actor_id() {
                        Ok(chat_actor_id) => {
                            let auto_task_message = protocol::ChatStateRequest::AddMessage {
                                message: Message {
                                    role: genai_types::messages::Role::User,
                                    content: vec![genai_types::MessageContent::Text {
                                        text: auto_message.to_string(),
                                    }],
                                },
                            };

                            let message_bytes = to_vec(&auto_task_message)
                                .map_err(|e| format!("Failed to serialize auto message: {}", e))?;

                            match send(chat_actor_id, &message_bytes) {
                                Ok(_) => {
                                    log("Auto task message sent successfully");

                                    // Request generation from chat-state actor
                                    let generation_request =
                                        protocol::ChatStateRequest::GenerateCompletion;
                                    let generation_request_bytes = to_vec(&generation_request)
                                        .map_err(|e| {
                                            format!("Failed to serialize generation request: {}", e)
                                        })?;

                                    match send(chat_actor_id, &generation_request_bytes) {
                                        Ok(_) => {
                                            log("Auto generation request sent successfully");
                                        }
                                        Err(e) => {
                                            let error_msg = format!(
                                                "Failed to send auto generation request: {:?}",
                                                e
                                            );
                                            log(&error_msg);
                                            return Ok((
                                                Some(to_vec(&git_state).unwrap_or_default()),
                                                (Some(
                                                    to_vec(&GitChatResponse::Error {
                                                        message: error_msg,
                                                    })
                                                    .unwrap_or_default(),
                                                ),),
                                            ));
                                        }
                                    }
                                }
                                Err(e) => {
                                    let error_msg =
                                        format!("Failed to send auto task message: {:?}", e);
                                    log(&error_msg);
                                    return Ok((
                                        Some(to_vec(&git_state).unwrap_or_default()),
                                        (Some(
                                            to_vec(&GitChatResponse::Error { message: error_msg })
                                                .unwrap_or_default(),
                                        ),),
                                    ));
                                }
                            }
                        }
                        Err(e) => {
                            let error_msg =
                                format!("Chat state actor not available for auto task: {}", e);
                            log(&error_msg);
                            return Ok((
                                Some(to_vec(&git_state).unwrap_or_default()),
                                (Some(
                                    to_vec(&GitChatResponse::Error { message: error_msg })
                                        .unwrap_or_default(),
                                ),),
                            ));
                        }
                    }
                } else {
                    log("No task specified, starting normal chat session");
                }

                GitChatResponse::Success
            }
            GitChatRequest::GetChatStateActorId => match git_state.get_chat_state_actor_id() {
                Ok(actor_id) => {
                    log(&format!("Returning chat state actor ID: {}", actor_id));
                    GitChatResponse::ChatStateActorId {
                        actor_id: actor_id.clone(),
                    }
                }
                Err(e) => {
                    log(&format!("Error getting chat state actor ID: {}", e));
                    GitChatResponse::Error { message: e }
                }
            },
            GitChatRequest::AddMessage { message } => {
                match git_state.get_chat_state_actor_id() {
                    Ok(chat_actor_id) => {
                        log(&format!(
                            "Forwarding message to chat state actor: {}",
                            chat_actor_id
                        ));

                        let add_message = protocol::ChatStateRequest::AddMessage {
                            message: message.clone(),
                        };

                        // Forward the message to the chat-state actor
                        let message_bytes = to_vec(&add_message)
                            .map_err(|e| format!("Failed to serialize message: {}", e))?;

                        match send(chat_actor_id, &message_bytes) {
                            Ok(_) => {
                                log("Message forwarded successfully");

                                // Request generation from chat-state actor
                                let generation_request_message =
                                    protocol::ChatStateRequest::GenerateCompletion;
                                let generation_request_bytes = to_vec(&generation_request_message)
                                    .map_err(|e| {
                                        format!("Failed to serialize generation request: {}", e)
                                    })?;

                                match send(chat_actor_id, &generation_request_bytes) {
                                    Ok(_) => {
                                        log("Generation request sent successfully");
                                        GitChatResponse::Success
                                    }
                                    Err(e) => {
                                        let error_msg =
                                            format!("Failed to send generation request: {:?}", e);
                                        log(&error_msg);
                                        GitChatResponse::Error { message: error_msg }
                                    }
                                }
                            }
                            Err(e) => {
                                let error_msg = format!("Failed to forward message: {:?}", e);
                                log(&error_msg);
                                GitChatResponse::Error { message: error_msg }
                            }
                        }
                    }
                    Err(e) => {
                        log(&format!("Error forwarding message: {}", e));
                        GitChatResponse::Error { message: e }
                    }
                }
            }
        };

        // Serialize the response
        let response_bytes =
            to_vec(&response).map_err(|e| format!("Failed to serialize response: {}", e))?;

        // Keep the same state (no changes needed)
        let current_state_bytes =
            to_vec(&git_state).map_err(|e| format!("Failed to serialize current state: {}", e))?;

        Ok((Some(current_state_bytes), (Some(response_bytes),)))
    }

    fn handle_channel_open(
        state: Option<Vec<u8>>,
        _params: (String, Vec<u8>),
    ) -> Result<(Option<Vec<u8>>, (ChannelAccept,)), String> {
        log("Git chat assistant: Channel open request");
        Ok((
            state,
            (ChannelAccept {
                accepted: true,
                message: None,
            },),
        ))
    }

    fn handle_channel_close(
        state: Option<Vec<u8>>,
        params: (String,),
    ) -> Result<(Option<Vec<u8>>,), String> {
        let (channel_id,) = params;
        log(&format!(
            "Git chat assistant: Channel closed: {}",
            channel_id
        ));
        Ok((state,))
    }

    fn handle_channel_message(
        state: Option<Vec<u8>>,
        params: (String, Vec<u8>),
    ) -> Result<(Option<Vec<u8>>,), String> {
        let (channel_id, _message) = params;
        log(&format!(
            "Git chat assistant: Received channel message on: {}",
            channel_id
        ));
        Ok((state,))
    }
}

// Helper functions
fn create_git_optimized_config(
    self_id: &str,
    current_directory: Option<&str>,
    config: &GitAssistantConfig,
) -> Value {
    log("Creating task-oriented git configuration...");

    // Build directory context if provided
    let directory_context = match current_directory {
        Some(dir) => {
            log(&format!("Including current directory context: {}", dir));
            format!("\n\nWORKING DIRECTORY: {}\nAll git operations should be performed in this directory.", dir)
        }
        None => {
            log("No current directory specified");
            String::new()
        }
    };

    // Build task context if provided
    let task_context = match config.task.as_deref() {
        Some("commit") => {
            log("Adding commit task context");
            "\n\nTASK: AUTOMATED COMMIT\n\
            Your task is to analyze the current repository and create appropriate commits:\n\
            \n\
            STEPS:\n\
            1. Check git status to identify changed files\n\
            2. Review changes using git diff to understand what was modified\n\
            3. Stage appropriate files for logical commits\n\
            4. Create meaningful, conventional commit messages\n\
            5. Execute commits with clear explanations\n\
            6. When all commits are complete, use the task_complete tool\n\
            \n\
            GOAL: Create clean, atomic commits with descriptive messages. \
            If there are multiple logical changes, create separate commits. \
            Always explain your reasoning and call task_complete when finished."
        }
        Some("review") => {
            log("Adding review task context");
            "\n\nTASK: CODE REVIEW\n\
            Your task is to thoroughly review the current code changes:\n\
            \n\
            STEPS:\n\
            1. Check git status and diff to understand all changes\n\
            2. Analyze code quality, style, and architecture\n\
            3. Identify potential bugs, security issues, or performance problems\n\
            4. Suggest specific improvements with examples\n\
            5. Provide constructive feedback on implementation choices\n\
            6. When review is complete, use the task_complete tool\n\
            \n\
            GOAL: Provide thorough, constructive code review that helps improve \
            code quality. Focus on being educational and actionable."
        }
        Some("rebase") => {
            log("Adding rebase task context");
            "\n\nTASK: INTERACTIVE REBASE\n\
            Your task is to help clean up the git history through rebase:\n\
            \n\
            STEPS:\n\
            1. Analyze current branch history and commit structure\n\
            2. Plan an appropriate rebase strategy\n\
            3. Guide through interactive rebase steps\n\
            4. Help resolve any merge conflicts that arise\n\
            5. Verify the final history is clean and logical\n\
            6. When rebase is complete, use the task_complete tool\n\
            \n\
            GOAL: Achieve a clean, linear git history while preserving \
            all important changes and maintaining code integrity."
        }
        Some("analyze") => {
            log("Adding analyze task context");
            "\n\nTASK: REPOSITORY ANALYSIS\n\
            Your task is to provide a comprehensive analysis of the repository:\n\
            \n\
            STEPS:\n\
            1. Examine repository structure and organization\n\
            2. Analyze recent commit history and patterns\n\
            3. Review current branch state and outstanding changes\n\
            4. Identify potential issues or improvements\n\
            5. Provide actionable recommendations\n\
            6. When analysis is complete, use the task_complete tool\n\
            \n\
            GOAL: Provide valuable insights about the repository state, \
            development patterns, and potential improvements."
        }
        Some("cleanup") => {
            log("Adding cleanup task context");
            "\n\nTASK: REPOSITORY CLEANUP\n\
            Your task is to clean up and organize the repository:\n\
            \n\
            STEPS:\n\
            1. Identify untracked files, stale branches, and clutter\n\
            2. Review .gitignore and suggest improvements\n\
            3. Clean up unnecessary files or directories\n\
            4. Organize commits if needed (squash, reorder)\n\
            5. Update documentation if outdated\n\
            6. When cleanup is complete, use the task_complete tool\n\
            \n\
            GOAL: Leave the repository in a clean, organized state \
            that follows best practices and is easy to navigate."
        }
        Some(task) => {
            log(&format!(
                "Unknown task type: {}, using default behavior",
                task
            ));
            ""
        }
        None => {
            log("No task specified");
            ""
        }
    };

    // Build completion instruction
    let completion_instruction = if config.task.is_some() {
        "\n\nIMPORTANT: When you have completed your assigned task, you MUST call the 'task_complete' tool \
        to signal that the work is finished. This allows the system to properly conclude the task session."
    } else {
        "\n\nNOTE: You have access to a 'task_complete' tool. Use it if the user explicitly asks you \
        to complete a specific task or when you finish a well-defined piece of work."
    };

    // Default git system prompt
    let default_git_system_prompt = format!(
        "You are a Git Task Assistant with access to git tools. You specialize in completing \
        specific git-related tasks efficiently and thoroughly.\n\
        \n\
        AVAILABLE CAPABILITIES:\n\
        - Git repository operations (status, diff, log, branch management)\n\
        - File staging and commit creation\n\
        - Branch operations and history analysis\n\
        - Code review and quality assessment\n\
        - Repository cleanup and organization\n\
        - Task completion signaling\n\
        \n\
        APPROACH:\n\
        - Always start by understanding the current repository state\n\
        - Break down complex tasks into clear steps\n\
        - Provide explanations for all git operations\n\
        - Follow git best practices and conventions\n\
        - Signal completion when tasks are finished{}{}{}",
        directory_context, task_context, completion_instruction
    );

    // Use custom system prompt if provided, otherwise use default with directory and task context
    let final_system_prompt = match &config.system_prompt {
        Some(custom_prompt) => {
            log("Using custom system prompt with context");
            format!(
                "{}{}{}{}",
                custom_prompt, directory_context, task_context, completion_instruction
            )
        }
        None => {
            log("Using default git system prompt with task context");
            default_git_system_prompt
        }
    };

    // Default model config
    let default_model_config = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "provider": "anthropic"
    });

    // Default MCP servers (git tools)
    let default_mcp_servers = serde_json::json!([
        {
            "actor_id": null,
            "actor": {
                "manifest_path": GIT_MCP_MANIFEST_PATH,
            },
            "tools": null
        },
        {
            "actor_id": null,
            "actor": {
                "manifest_path": TASK_MONITOR_MANIFEST_PATH,
                "init_state": {
                    "management_actor": self_id,
                }
            },
            "tools": null
        }
    ]);

    // Build the configuration with overrides
    let model_config = config
        .model_config
        .as_ref()
        .unwrap_or(&default_model_config);

    // Adjust temperature based on task type
    let default_temperature = match config.task.as_deref() {
        Some("commit") => 0.3,  // More deterministic for commit messages
        Some("review") => 0.5,  // Balanced for analysis
        Some("rebase") => 0.2,  // Very precise for history operations
        Some("analyze") => 0.6, // Slightly creative for insights
        Some("cleanup") => 0.3, // Methodical approach
        _ => 0.7,               // Default for general assistance
    };

    let temperature = config.temperature.unwrap_or(default_temperature);
    let max_tokens = config.max_tokens.unwrap_or(8192);

    // Update title based on task
    let default_title = match config.task.as_deref() {
        Some("commit") => "Git Commit Assistant",
        Some("review") => "Git Code Review Assistant",
        Some("rebase") => "Git Rebase Assistant",
        Some("analyze") => "Git Analysis Assistant",
        Some("cleanup") => "Git Cleanup Assistant",
        Some(_) => "Git Task Assistant",
        None => "Git Assistant",
    };

    let title = config.title.as_deref().unwrap_or(default_title);
    let default_description = format!(
        "AI assistant for git {} tasks",
        config.task.as_deref().unwrap_or("management")
    );
    let description = config
        .description
        .as_deref()
        .unwrap_or(&default_description);
    let mcp_servers = config.mcp_servers.as_ref().unwrap_or(&default_mcp_servers);

    log(&format!("Using model: {:?}", model_config));
    log(&format!("Using temperature: {}", temperature));
    log(&format!("Using max_tokens: {}", max_tokens));
    log(&format!("Using title: {}", title));

    // Build the final configuration
    let mut final_config = serde_json::json!({
        "model_config": model_config,
        "temperature": temperature,
        "max_tokens": max_tokens,
        "system_prompt": final_system_prompt,
        "title": title,
        "description": description,
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

    log(&format!("Created final git config: {}", final_config));
    final_config
}

fn spawn_chat_state_actor(chat_config: &Value) -> Result<String, String> {
    log("Spawning chat-state actor...");

    // Create initial state for chat-state actor
    let initial_state = serde_json::json!({
        "config": chat_config
    });

    let initial_state_bytes = to_vec(&initial_state)
        .map_err(|e| format!("Failed to serialize chat-state config: {}", e))?;

    // Spawn the actor
    match spawn(CHAT_STATE_MANIFEST_PATH, Some(&initial_state_bytes)) {
        Ok(actor_id) => {
            log(&format!(
                "Successfully spawned chat-state actor: {}",
                actor_id
            ));
            Ok(actor_id)
        }
        Err(e) => {
            log(&format!("Failed to spawn chat-state actor: {:?}", e));
            Err(format!("Spawn failed: {:?}", e))
        }
    }
}

bindings::export!(Component with_types_in bindings);
