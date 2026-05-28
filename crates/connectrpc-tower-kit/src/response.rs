//! Build a Connect-protocol error response from any [`ConnectError`].
//!
//! Every short-circuiting Layer in the family needs the same: take a
//! `ConnectError`, produce a `Response<ConnectRpcBody>` the inner
//! service would have produced. The Connect spec encodes errors as an
//! HTTP response with `application/json` body
//! `{"code": "<code>", "message": "..."}` — `ConnectError::to_json()`
//! and `::http_status()` handle the encoding. This module just wires
//! response builder + headers + trailers correctly so callers don't
//! re-derive the same dance per middleware.

use bytes::Bytes;
use connectrpc::{ConnectError, ConnectRpcBody};
use http::{HeaderName, Response, StatusCode};
use http_body_util::Full;

/// Build a `Response<ConnectRpcBody>` from a `ConnectError`.
///
/// Use this whenever a short-circuiting Layer wants to reject a request
/// without invoking the inner service. The result is the same wire
/// format `ConnectRpcService` would have produced if the handler had
/// returned `Err(err)`.
///
/// # Example
///
/// ```ignore
/// use connectrpc::ConnectError;
/// use connectrpc_tower_kit::deny_response;
///
/// let resp = deny_response(ConnectError::permission_denied("cedar denied"));
/// // → Response<ConnectRpcBody> with 403 status and Connect-protocol JSON body.
/// ```
pub fn deny_response(err: ConnectError) -> Response<ConnectRpcBody> {
    let body_bytes = err.to_json();
    let status = err.http_status();

    let mut builder = Response::builder()
        .status(status)
        .header(http::header::CONTENT_TYPE, "application/json");

    // Surface ConnectError-attached response headers (rare but allowed).
    for (k, v) in err.response_headers() {
        builder = builder.header(k, v);
    }

    // Connect protocol trailers travel as `trailer-<name>` headers in
    // the unary response (no real HTTP trailers — fetch can't read them
    // in browser ConnectRPC clients).
    for (k, v) in err.trailers() {
        let prefixed = format!("trailer-{}", k.as_str());
        if let Ok(name) = HeaderName::try_from(prefixed) {
            builder = builder.header(name, v);
        }
    }

    builder
        .body(ConnectRpcBody::Full(Full::new(body_bytes)))
        .unwrap_or_else(|_| {
            // Fallback if the builder rejects something — should never
            // happen with the static headers above; the `expect` documents
            // that invariant rather than hiding a default.
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(ConnectRpcBody::Full(Full::new(Bytes::new())))
                .expect("static infallible builder")
        })
}
