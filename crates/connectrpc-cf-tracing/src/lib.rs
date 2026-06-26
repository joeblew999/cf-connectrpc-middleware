//! CF-Workers-aware tracing layer for Connect-RPC services.
//!
//! ## What this crate does
//!
//! Provides [`TracingLayer`] — a transparent `tower::Layer` that opens
//! a [`tracing::Span`] around every Connect-RPC call. The span carries
//! Cloudflare-specific runtime metadata (colo, country, ASN, TLS
//! cipher, HTTP protocol, `cf-ray`) as structured fields, plus the
//! resolved Connect-RPC procedure path.
//!
//! Pairs with `connectrpc-cedar` and friends — usually the outermost
//! layer on the stack, so the span wraps everything downstream
//! (authz decisions, request bodies, errors).
//!
//! ## CF Workers compatibility
//!
//! - **Builds on `wasm32-unknown-unknown`**: yes. The crate has no
//!   `worker` dep; it just consumes a `CfFields` value the consumer
//!   builds from whatever source they have.
//! - **CF binding required**: none.
//! - **CF runtime data read**: `request.cf.*` + `cf-ray` header.
//!   Available on every inbound request a Worker handles.
//! - **Where the consumer wires it**: in a closure that takes
//!   `&http::Request<B>` and returns a [`CfFields`]. The standard
//!   wiring on CF Workers reads `req.extensions().get::<worker::Cf>()`
//!   and pulls headers — see the README for the 8-line snippet.
//!
//! ## Why this is a *transparent* tower::Layer
//!
//! Tracing never rejects requests. It only observes. So this layer is
//! generic over `S` with `type Response = S::Response` (no `ConnectRpcBody`
//! pinning, no `Error = Infallible` requirement). That's the same
//! shape as a transparent `RequestIdLayer` / `AuthLayer`.
//!
//! The kit's `Rollout` trait is **not** used here because tracing has
//! no "off" mode worth toggling. If a future variant wants sampling,
//! it adopts `Rollout` with a `Sample`/`All` enum.

pub mod extract;
pub mod layer;

pub use extract::{CfFields, CfFieldsExtractor};
pub use layer::{TracingFuture, TracingLayer, TracingService};
