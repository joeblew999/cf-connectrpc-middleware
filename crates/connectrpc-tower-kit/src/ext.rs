//! Canonical names for `http::Request::extensions()` entries.
//!
//! This module **documents convention, it does not define types.**
//! Different consumers have different concrete session types
//! (macaroons vs JWTs vs session cookies). The kit can't force a
//! shape. What it *can* do is standardize the *names* so middlewares
//! compose: if every layer agrees that "the session" goes in via
//! `Session` (the name, not a specific struct), then anyone's
//! AuthN layer (`OidcLayer` or `SessionLayer`) works with anyone's `CedarLayer`.
//! (Formerly the consumer-side `SessionContext` in the example workers.)
//!
//! # The convention
//!
//! | Name | Inserted by | Consumed by |
//! | --- | --- | --- |
//! | `Session` | an AuthN layer (`OidcLayer` or `SessionLayer`) (or equivalent — anything that verifies a bearer token) | `CedarLayer`, `CedarInterceptor`, handler `require_session(ctx)?` |
//! | `RequestId` | `RequestIdLayer` (or upstream `x-request-id` propagation) | tracing layer, logging interceptor, response header echoer |
//! | `GeoContext` | `CFContextLayer` (reads `request.cf.{country,colo,asn,...}`) | tracing fields, geo-gated authz |
//! | `TraceContext` | `CFTraceContextLayer` (W3C `traceparent` / CF `cf-ray`) | distributed-tracing interceptor |
//!
//! Each consumer crate defines its own concrete struct (`pub struct
//! Session { ... }` in `example-multitenant-worker`,
//! `pub struct Session { ... }` in `EdgeReplica`) and ALSO
//! exports the same name. That way, swapping the auth implementation
//! doesn't break Cedar.
//!
//! # Pattern: "soft middleware + handler backstop"
//!
//! Layers insert into extensions on success, do **nothing** on
//! failure (no rejection). Handlers call `require_session(ctx)?`
//! themselves to surface `Unauthenticated`. This way unauthenticated
//! endpoints (login, signup, OAuth start) work without an "is_public"
//! allowlist, and middleware bugs degrade to "401 from handler" rather
//! than "everything 401".
//!
//! `CedarLayer` follows the same rule: missing `Session` →
//! pass through (anonymous request, handler decides). Present
//! `Session` + `Decision::Deny` in `Mode::Enforce` → short-circuit.
//!
//! # When you need typed access
//!
//! Pull the typed value out with `ctx.extensions().get::<YourType>()`
//! inside a handler, or `req.extensions().get::<YourType>()` inside
//! another layer. The kit doesn't ship the types because their fields
//! belong to the consumer.

// Module intentionally has no code — documentation only.
