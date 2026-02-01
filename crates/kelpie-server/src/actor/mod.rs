//! Actor-based agent implementation
//!
//! TigerStyle: AgentActor implements the Actor trait from kelpie-runtime,
//! providing single activation guarantee, automatic lifecycle, and state persistence.

pub mod agent_actor;
pub mod dispatcher_adapter;
pub mod llm_trait;
pub mod registry_actor;
pub mod state;

pub use agent_actor::{
    AgentActor, AgentContinuation, ArchivalDeleteRequest, ArchivalInsertRequest,
    ArchivalInsertResponse, ArchivalSearchRequest, ArchivalSearchResponse, CallContextInfo,
    ContinueWithToolResultsRequest, ConversationSearchDateRequest, ConversationSearchRequest,
    ConversationSearchResponse, CoreMemoryReplaceRequest, GetBlockRequest, GetBlockResponse,
    HandleMessageFullRequest, HandleMessageFullResponse, HandleMessageResult, ListMessagesRequest,
    ListMessagesResponse, PendingToolCall, ToolResult,
};
pub use dispatcher_adapter::{DispatcherAdapter, AGENT_ACTOR_NAMESPACE};
pub use llm_trait::{LlmClient, LlmMessage, LlmResponse, LlmToolCall, RealLlmAdapter, StreamChunk};
pub use registry_actor::{
    GetRequest, GetResponse, ListRequest, ListResponse, RegisterRequest, RegisterResponse,
    RegistryActor, RegistryActorState, UnregisterRequest, UnregisterResponse,
};
pub use state::AgentActorState;
