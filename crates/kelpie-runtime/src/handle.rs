//! Actor handles for external invocations
//!
//! TigerStyle: Location-transparent references with explicit error handling.

use crate::dispatcher::DispatcherHandle;
use bytes::Bytes;
use kelpie_core::actor::{ActorId, ActorRef};
use kelpie_core::error::{Error, Result};
use std::time::Duration;

/// Handle to invoke an actor
///
/// Provides a location-transparent reference to an actor. The handle can be
/// cloned and shared across tasks/threads.
#[derive(Clone)]
pub struct ActorHandle<R: kelpie_core::Runtime> {
    /// The actor's reference
    actor_ref: ActorRef,
    /// Dispatcher handle for routing
    dispatcher: DispatcherHandle<R>,
    /// Default timeout for invocations
    default_timeout: Option<Duration>,
}

impl<R: kelpie_core::Runtime> ActorHandle<R> {
    /// Create a new actor handle
    pub fn new(actor_ref: ActorRef, dispatcher: DispatcherHandle<R>) -> Self {
        Self {
            actor_ref,
            dispatcher,
            default_timeout: None,
        }
    }

    /// Create a handle with a default timeout
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.default_timeout = Some(timeout);
        self
    }

    /// Get the actor's reference
    pub fn actor_ref(&self) -> &ActorRef {
        &self.actor_ref
    }

    /// Get the actor's ID
    pub fn id(&self) -> &ActorId {
        &self.actor_ref.id
    }

    /// Invoke the actor with an operation and payload
    pub async fn invoke(&self, operation: impl Into<String>, payload: Bytes) -> Result<Bytes> {
        let operation = operation.into();

        match self.default_timeout {
            Some(timeout) => {
                let runtime = kelpie_core::current_runtime();
                kelpie_core::Runtime::timeout(
                    &runtime,
                    timeout,
                    self.invoke_inner(&operation, payload),
                )
                .await
                .map_err(|_| Error::OperationTimedOut {
                    operation: operation.clone(),
                    timeout_ms: timeout.as_millis() as u64,
                })?
            }
            None => self.invoke_inner(&operation, payload).await,
        }
    }

    /// Internal invoke without timeout
    async fn invoke_inner(&self, operation: &str, payload: Bytes) -> Result<Bytes> {
        self.dispatcher
            .invoke(self.actor_ref.id.clone(), operation.to_string(), payload)
            .await
    }

    /// Invoke with a typed request and response
    ///
    /// Serializes the request to JSON, invokes the actor, and deserializes the response.
    pub async fn request<Req, Resp>(
        &self,
        operation: impl Into<String>,
        request: &Req,
    ) -> Result<Resp>
    where
        Req: serde::Serialize,
        Resp: serde::de::DeserializeOwned,
    {
        let payload = serde_json::to_vec(request).map_err(|e| Error::Internal {
            message: format!("failed to serialize request: {}", e),
        })?;

        let response = self.invoke(operation, Bytes::from(payload)).await?;

        serde_json::from_slice(&response).map_err(|e| Error::Internal {
            message: format!("failed to deserialize response: {}", e),
        })
    }

    /// Send a fire-and-forget message (no response expected)
    pub async fn send(&self, operation: impl Into<String>, payload: Bytes) -> Result<()> {
        self.invoke(operation, payload).await?;
        Ok(())
    }

    /// Deactivate the actor
    ///
    /// The actor will be reactivated on the next invocation.
    pub async fn deactivate(&self) -> Result<()> {
        self.dispatcher.deactivate(self.actor_ref.id.clone()).await
    }
}

/// Builder for creating actor handles
pub struct ActorHandleBuilder<R: kelpie_core::Runtime> {
    dispatcher: DispatcherHandle<R>,
}

impl<R: kelpie_core::Runtime> ActorHandleBuilder<R> {
    /// Create a new builder
    pub fn new(dispatcher: DispatcherHandle<R>) -> Self {
        Self { dispatcher }
    }

    /// Create a handle for the given actor ID
    pub fn for_actor(&self, actor_id: ActorId) -> ActorHandle<R> {
        ActorHandle::new(ActorRef::new(actor_id), self.dispatcher.clone())
    }

    /// Create a handle for the given namespace and ID
    pub fn for_parts(
        &self,
        namespace: impl Into<String>,
        id: impl Into<String>,
    ) -> Result<ActorHandle<R>> {
        let actor_id = ActorId::new(namespace, id)?;
        Ok(self.for_actor(actor_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatcher::{CloneFactory, Dispatcher, DispatcherConfig};
    use async_trait::async_trait;
    use kelpie_core::actor::{Actor, ActorContext};
    use kelpie_core::Runtime;
    use kelpie_storage::MemoryKV;
    use std::sync::Arc;

    #[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
    struct EchoState;

    #[derive(Clone)]
    struct EchoActor;

    #[async_trait]
    impl Actor for EchoActor {
        type State = EchoState;

        async fn invoke(
            &self,
            _ctx: &mut ActorContext<Self::State>,
            operation: &str,
            payload: Bytes,
        ) -> Result<Bytes> {
            match operation {
                "echo" => Ok(payload),
                "upper" => {
                    let text = String::from_utf8_lossy(&payload);
                    Ok(Bytes::from(text.to_uppercase()))
                }
                _ => Err(Error::InvalidOperation {
                    operation: operation.to_string(),
                }),
            }
        }
    }

    #[tokio::test]
    async fn test_actor_handle_basic() {
        use kelpie_core::TokioRuntime;

        let factory = Arc::new(CloneFactory::new(EchoActor));
        let kv = Arc::new(MemoryKV::new());
        let config = DispatcherConfig::default();
        let runtime = TokioRuntime;

        let mut dispatcher = Dispatcher::new(factory, kv, config, runtime.clone());
        let handle = dispatcher.handle();

        let dispatcher_task = runtime.spawn(async move {
            dispatcher.run().await;
        });

        let actor_id = ActorId::new("test", "echo-1").unwrap();
        let actor_handle = ActorHandle::new(ActorRef::new(actor_id), handle.clone());

        let result = actor_handle
            .invoke("echo", Bytes::from("hello"))
            .await
            .unwrap();
        assert_eq!(result, Bytes::from("hello"));

        let result = actor_handle
            .invoke("upper", Bytes::from("hello"))
            .await
            .unwrap();
        assert_eq!(result, Bytes::from("HELLO"));

        handle.shutdown().await.unwrap();
        dispatcher_task.await.unwrap();
    }

    #[tokio::test]
    async fn test_actor_handle_builder() {
        use kelpie_core::TokioRuntime;

        let factory = Arc::new(CloneFactory::new(EchoActor));
        let kv = Arc::new(MemoryKV::new());
        let config = DispatcherConfig::default();
        let runtime = TokioRuntime;

        let mut dispatcher = Dispatcher::new(factory, kv, config, runtime.clone());
        let dispatcher_handle = dispatcher.handle();

        let dispatcher_task = runtime.spawn(async move {
            dispatcher.run().await;
        });

        let builder = ActorHandleBuilder::new(dispatcher_handle.clone());
        let actor_handle = builder.for_parts("test", "echo-2").unwrap();

        let result = actor_handle
            .invoke("echo", Bytes::from("test"))
            .await
            .unwrap();
        assert_eq!(result, Bytes::from("test"));

        dispatcher_handle.shutdown().await.unwrap();
        dispatcher_task.await.unwrap();
    }

    #[tokio::test]
    async fn test_actor_handle_timeout() {
        use kelpie_core::TokioRuntime;

        let factory = Arc::new(CloneFactory::new(EchoActor));
        let kv = Arc::new(MemoryKV::new());
        let config = DispatcherConfig::default();
        let runtime = TokioRuntime;

        let mut dispatcher = Dispatcher::new(factory, kv, config, runtime.clone());
        let dispatcher_handle = dispatcher.handle();

        let dispatcher_task = runtime.spawn(async move {
            dispatcher.run().await;
        });

        let actor_id = ActorId::new("test", "echo-timeout").unwrap();
        let actor_handle = ActorHandle::new(ActorRef::new(actor_id), dispatcher_handle.clone())
            .with_timeout(Duration::from_secs(5));

        // This should succeed within timeout
        let result = actor_handle
            .invoke("echo", Bytes::from("fast"))
            .await
            .unwrap();
        assert_eq!(result, Bytes::from("fast"));

        dispatcher_handle.shutdown().await.unwrap();
        dispatcher_task.await.unwrap();
    }

    #[derive(serde::Serialize, serde::Deserialize)]
    struct EchoRequest {
        message: String,
    }

    #[derive(serde::Serialize, serde::Deserialize)]
    struct EchoResponse {
        message: String,
    }

    #[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
    struct JsonEchoState;

    #[derive(Clone)]
    struct JsonEchoActor;

    #[async_trait]
    impl Actor for JsonEchoActor {
        type State = JsonEchoState;

        async fn invoke(
            &self,
            _ctx: &mut ActorContext<Self::State>,
            operation: &str,
            payload: Bytes,
        ) -> Result<Bytes> {
            match operation {
                "echo" => {
                    let req: EchoRequest =
                        serde_json::from_slice(&payload).map_err(|e| Error::Internal {
                            message: format!("invalid request: {}", e),
                        })?;
                    let resp = EchoResponse {
                        message: req.message.to_uppercase(),
                    };
                    let bytes = serde_json::to_vec(&resp).map_err(|e| Error::Internal {
                        message: format!("serialization failed: {}", e),
                    })?;
                    Ok(Bytes::from(bytes))
                }
                _ => Err(Error::InvalidOperation {
                    operation: operation.to_string(),
                }),
            }
        }
    }

    #[tokio::test]
    async fn test_actor_handle_typed_request() {
        use kelpie_core::TokioRuntime;

        let factory = Arc::new(CloneFactory::new(JsonEchoActor));
        let kv = Arc::new(MemoryKV::new());
        let config = DispatcherConfig::default();
        let runtime = TokioRuntime;

        let mut dispatcher = Dispatcher::new(factory, kv, config, runtime.clone());
        let dispatcher_handle = dispatcher.handle();

        let dispatcher_task = runtime.spawn(async move {
            dispatcher.run().await;
        });

        let actor_id = ActorId::new("test", "json-echo").unwrap();
        let actor_handle = ActorHandle::new(ActorRef::new(actor_id), dispatcher_handle.clone());

        let request = EchoRequest {
            message: "hello world".to_string(),
        };
        let response: EchoResponse = actor_handle.request("echo", &request).await.unwrap();
        assert_eq!(response.message, "HELLO WORLD");

        dispatcher_handle.shutdown().await.unwrap();
        dispatcher_task.await.unwrap();
    }
}
