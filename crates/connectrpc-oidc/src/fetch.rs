//! JWKS fetching helpers. The verifier ([`crate::JwksVerifier`]) takes
//! already-fetched JSON; this module provides the host-specific fetch so a
//! consumer can wire it in one line.
//!
//! Only the Worker path lives here (behind `worker-jwks`) — on `wasm32` you
//! MUST go through `worker::Fetch` (no sockets). On native, callers use their
//! own HTTP client (`reqwest`/`ureq`), so there's nothing host-specific to
//! provide and pulling an HTTP stack into this crate would only bloat it.
//!
//! Fetch JWKS ONCE at Worker boot and reuse the built verifier — never per
//! request (that adds a subrequest + latency to every call).

#[cfg(feature = "worker-jwks")]
pub use worker_impl::fetch_jwks;

#[cfg(feature = "worker-jwks")]
mod worker_impl {
    use worker::{Fetch, Request};

    /// Fetch the JWKS document from `url` via `worker::Fetch`.
    ///
    /// The returned future is `!Send` (it holds JS values); inside a
    /// connectrpc 0.6 handler chain `.into_send()` before `.await` to satisfy
    /// the `Send` bound. At Worker *boot* (before the request loop) that isn't
    /// required — await it directly.
    pub async fn fetch_jwks(url: &str) -> worker::Result<String> {
        let req = Request::new(url, worker::Method::Get)?;
        let mut resp = Fetch::Request(req).send().await?;
        resp.text().await
    }
}
