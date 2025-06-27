use genai_types::Message;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

// Actor API request structures
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
pub enum McpActorRequest {
    ToolsList {},
    ToolsCall { name: String, args: Value },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct McpResponse {
    pub jsonrpc: String,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<McpError>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct McpError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Messages received by the chat-state actor
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum ChatStateRequest {
    #[serde(rename = "add_message")]
    AddMessage { message: Message },
    #[serde(rename = "generate_completion")]
    GenerateCompletion,
}

/// Data associated with the response
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum ChatStateResponse {
    #[serde(rename = "success")]
    Success,

    #[serde(rename = "error")]
    Error { error: ErrorInfo },
}

/// Error information
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ErrorInfo {
    /// Error code
    pub code: String,

    /// Human-readable error message
    pub message: String,

    /// Additional error details
    pub details: Option<HashMap<String, String>>,
}
