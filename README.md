# Task Manager Actor

A general-purpose Theater actor that orchestrates task execution by managing chat-state actors with configurable AI models, system prompts, and tool access.

## Purpose

The Task Manager is a **configuration-driven orchestrator** that:
1. Accepts task configuration (system prompt, AI settings, tools)
2. Spawns a chat-state actor with the specified configuration
3. Manages task execution and completion workflow
4. Provides a clean abstraction for task-oriented AI interactions

## Features

- **Configuration-Driven** - Fully customizable through initial state
- **Model Agnostic** - Supports any AI model/provider
- **Tool Integration** - Configurable MCP tool actors
- **Task Completion** - Built-in task completion workflow
- **General Purpose** - Can handle any task type

## Architecture

```
calling-agent → task-manager → chat-state + tools
    (config)    (orchestrator)   (execution)
```

## Configuration

The actor accepts a `TaskManagerConfig` in its initial state:

```rust
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
}
```

### Example Configuration

```json
{
  "system_prompt": "You are a git assistant. Help the user with git operations.",
  "initial_message": "I'll help you with git. Let me check the repository status.",
  "model_config": {
    "model": "claude-sonnet-4-20250514",
    "provider": "anthropic"
  },
  "temperature": 0.3,
  "max_tokens": 4096,
  "mcp_servers": [
    {
      "actor": {
        "manifest_path": "path/to/git-mcp-actor/manifest.toml"
      }
    }
  ],
  "auto_exit_on_completion": true
}
```

## Default Behavior

If no configuration is provided, the task-manager uses sensible defaults:
- **System Prompt**: Generic task assistant
- **Model**: Claude Sonnet 4
- **Temperature**: 0.7
- **Max Tokens**: 8192
- **Tools**: Task monitor (for completion signaling)

## Protocol

The actor implements the same protocol as other chat proxy actors:

### `GetChatStateActorId`
Returns the actor ID of the spawned chat-state actor.

### `AddMessage`
Forwards a message to the chat-state actor.

### `StartChat`
Initiates the chat and sends the initial message if configured.

## Usage Examples

### Git Workflow Agent
```json
{
  "system_prompt": "You are a git commit assistant. Analyze the repository and create appropriate commits.",
  "model_config": { "model": "gemini-2.0-flash", "provider": "google" },
  "temperature": 0.3,
  "mcp_servers": [{ "actor": { "manifest_path": "git-mcp-actor/manifest.toml" } }]
}
```

### Code Review Agent
```json
{
  "system_prompt": "You are a code review assistant. Analyze code changes and provide feedback.",
  "model_config": { "model": "claude-sonnet-4-20250514", "provider": "anthropic" },
  "temperature": 0.5,
  "mcp_servers": [{ "actor": { "manifest_path": "git-mcp-actor/manifest.toml" } }]
}
```

### Data Analysis Agent
```json
{
  "system_prompt": "You are a data analysis assistant. Help analyze and visualize data.",
  "model_config": { "model": "claude-sonnet-4-20250514", "provider": "anthropic" },
  "temperature": 0.6,
  "mcp_servers": [
    { "actor": { "manifest_path": "file-mcp-actor/manifest.toml" } },
    { "actor": { "manifest_path": "python-mcp-actor/manifest.toml" } }
  ]
}
```

## Benefits Over Domain-Specific Actors

1. **Single Actor to Maintain** - One well-tested orchestrator
2. **Consistent Interface** - Same protocol across all task types
3. **Easy Extension** - New task types require only configuration
4. **Better Testing** - Focused testing on orchestration logic
5. **Cleaner Architecture** - Clear separation of concerns

## Building

```bash
cargo component build --release
```

## Migration from Domain-Specific Actors

To migrate from actors like `git-chat-assistant`:

1. Replace the domain-specific actor with `task-manager`
2. Move the system prompt and configuration logic to the calling agent
3. Update the manifest path in configurations
4. Remove domain-specific actors once migration is complete

The task-manager provides the same orchestration capabilities with much greater flexibility and reusability.
