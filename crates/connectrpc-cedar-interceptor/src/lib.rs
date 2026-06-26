//! # connectrpc-cedar-interceptor
//!
//! Cedar policy authorization on the **`connectrpc::Interceptor`** surface
//! — the body-aware sibling of [`connectrpc-cedar`](connectrpc_cedar)'s
//! `tower::Layer`. This is the body-aware authz surface, built on the
//! `Interceptor` trait from connectrpc 0.7.
//!
//! ## Layer vs Interceptor — when to use which
//!
//! | | `connectrpc-cedar` (Layer) | `connectrpc-cedar-interceptor` (this) |
//! |---|---|---|
//! | Surface | `tower::Layer` over `ConnectRpcService` | `connectrpc::Interceptor` on the service |
//! | Runs | before envelope decode | after envelope decode |
//! | Sees | raw `http::Request` (headers + extensions) | `RequestContext` + **decoded message body** |
//! | Denial | hand-built `deny_response(..)` body | return `Err(ConnectError::permission_denied(..))` |
//! | Best for | path/header authz, cheapest rejection | authz that needs a field from the request body |
//!
//! Both share the same [`CedarAuthorizer`], [`CedarRequest`], and [`Mode`]
//! — pick the surface per the decision data you need. You can even run the
//! Layer in `Enforce` for coarse path authz and this interceptor in
//! `Shadow` for body-aware rules during a rollout.
//!
//! ## Quick start
//!
//! ```ignore
//! use std::sync::Arc;
//! use connectrpc_cedar_interceptor::{CedarInterceptor, CedarRequest};
//!
//! let authorizer = Arc::new(CedarAuthorizer::from_str(SCHEMA, POLICIES)?);
//!
//! let interceptor = CedarInterceptor::shadow(authorizer, |req: &connectrpc::UnaryRequest| {
//!     let session = req.ctx.extensions().get::<Session>()?;
//!     // body-aware: read a field off the decoded request message
//!     // let org = req.payload.message::<CreateOrgRequest>().ok()?;
//!     Some(CedarRequest { /* principal, action, resource, context */ })
//! }).skip_paths(["/workers.health.v1.HealthService/Check"]);
//!
//! let service = ConnectRpcService::new(router).with_interceptor(interceptor);
//! ```

pub mod extract;
pub mod interceptor;

pub use extract::CedarUnaryExtractor;
pub use interceptor::CedarInterceptor;

// Re-export the pieces a consumer needs so depending on this one crate is
// enough (no separate connectrpc-cedar / cedar-policy dep for simple uses).
pub use connectrpc_cedar::{CedarAuthorizer, CedarAuthorizerError, CedarRequest, Mode};
pub use connectrpc_cedar::{Context, Decision, EntityUid};
