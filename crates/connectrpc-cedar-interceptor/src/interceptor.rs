//! `connectrpc::Interceptor` wrapping a `CedarAuthorizer`.
//!
//! This is the Interceptor-surface sibling of `connectrpc-cedar`'s
//! `CedarLayer`. It evaluates the same Cedar (principal, action,
//! resource, context) tuple against the same [`CedarAuthorizer`] and
//! uses the same [`Mode`] (`Shadow` / `Enforce`) — but on the RPC
//! surface, after envelope decode, so the extractor can read the decoded
//! request body (see [`crate::extract`]).
//!
//! ## Shadow vs Enforce
//!
//! Identical semantics to the Layer:
//! - [`Mode::Shadow`] — evaluate + log via `tracing`, always pass through
//!   (`next.run(req)`).
//! - [`Mode::Enforce`] — pass through on `Allow`; on `Deny` short-circuit
//!   by returning `Err(ConnectError::permission_denied(..))`. The Connect
//!   framework encodes that into the response — no `deny_response` builder
//!   needed here (that was a Layer concern, because the Layer's
//!   `Error = Infallible` forced denials into the body by hand).

use std::sync::Arc;

use cedar_policy::Decision;
use connectrpc::{ConnectError, Interceptor, Next, UnaryRequest, UnaryResponse, async_trait};
use tracing::{info, warn};

use connectrpc_cedar::{CedarAuthorizer, Mode};

use crate::extract::CedarUnaryExtractor;

/// Cedar authorization on the `connectrpc::Interceptor` surface.
///
/// Construct via [`CedarInterceptor::shadow`] / [`CedarInterceptor::enforce`];
/// register on a `ConnectRpcService` with `.with_interceptor(..)`.
pub struct CedarInterceptor<E> {
    authorizer: Arc<CedarAuthorizer>,
    extractor: Arc<E>,
    mode: Mode,
    skip_paths: Arc<Vec<String>>,
}

impl<E> Clone for CedarInterceptor<E> {
    fn clone(&self) -> Self {
        Self {
            authorizer: Arc::clone(&self.authorizer),
            extractor: Arc::clone(&self.extractor),
            mode: self.mode,
            skip_paths: Arc::clone(&self.skip_paths),
        }
    }
}

impl<E> CedarInterceptor<E> {
    /// Shadow-mode: evaluate + log, never reject.
    pub fn shadow(authorizer: Arc<CedarAuthorizer>, extractor: E) -> Self {
        Self {
            authorizer,
            extractor: Arc::new(extractor),
            mode: Mode::Shadow,
            skip_paths: Arc::new(Vec::new()),
        }
    }

    /// Enforce-mode: reject on `Decision::Deny`.
    pub fn enforce(authorizer: Arc<CedarAuthorizer>, extractor: E) -> Self {
        Self {
            authorizer,
            extractor: Arc::new(extractor),
            mode: Mode::Enforce,
            skip_paths: Arc::new(Vec::new()),
        }
    }

    /// Procedures (Connect paths, e.g. `/pkg.v1.Svc/Health`) the
    /// interceptor skips entirely — no Cedar evaluation, straight to
    /// `next`. For anonymous endpoints (health, OAuth callback).
    pub fn skip_paths<I, S>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.skip_paths = Arc::new(paths.into_iter().map(Into::into).collect());
        self
    }
}

#[async_trait]
impl<E: CedarUnaryExtractor> Interceptor for CedarInterceptor<E> {
    async fn intercept_unary(
        &self,
        req: UnaryRequest,
        next: Next<'_>,
    ) -> Result<UnaryResponse, ConnectError> {
        // Skip-list check. `path()` borrows `req`; scope it so the borrow
        // ends before we move `req` into `next.run`.
        if let Some(path) = req.ctx.path() {
            if self.skip_paths.iter().any(|p| p == path) {
                return next.run(req).await;
            }
        }

        // Build the Cedar tuple from the decoded request. `None` => no
        // Cedar-relevant shape; pass through.
        let Some(cedar_req) = self.extractor.extract(&req) else {
            return next.run(req).await;
        };

        let (decision, reasons) = self.authorizer.is_authorized(
            &cedar_req.principal,
            &cedar_req.action,
            &cedar_req.resource,
            cedar_req.context,
        );

        // Log every decision (both modes) with the same shape as the
        // Layer, plus `surface = "interceptor"` so logs are distinguishable
        // when both surfaces run during a migration.
        match (self.mode, decision) {
            (Mode::Shadow, Decision::Allow) => info!(
                target: "connectrpc_cedar",
                surface = "interceptor",
                mode = "shadow",
                decision = "ALLOW",
                principal = %cedar_req.principal,
                action = %cedar_req.action,
                resource = %cedar_req.resource,
                reasons = ?reasons,
            ),
            (Mode::Shadow, Decision::Deny) => warn!(
                target: "connectrpc_cedar",
                surface = "interceptor",
                mode = "shadow",
                decision = "DENY",
                note = "would-have-rejected in enforce mode",
                principal = %cedar_req.principal,
                action = %cedar_req.action,
                resource = %cedar_req.resource,
                reasons = ?reasons,
            ),
            (Mode::Enforce, Decision::Allow) => info!(
                target: "connectrpc_cedar",
                surface = "interceptor",
                mode = "enforce",
                decision = "ALLOW",
                principal = %cedar_req.principal,
                action = %cedar_req.action,
                resource = %cedar_req.resource,
                reasons = ?reasons,
            ),
            (Mode::Enforce, Decision::Deny) => warn!(
                target: "connectrpc_cedar",
                surface = "interceptor",
                mode = "enforce",
                decision = "DENY",
                principal = %cedar_req.principal,
                action = %cedar_req.action,
                resource = %cedar_req.resource,
                reasons = ?reasons,
            ),
        }

        match (self.mode, decision) {
            (Mode::Shadow, _) | (Mode::Enforce, Decision::Allow) => next.run(req).await,
            (Mode::Enforce, Decision::Deny) => {
                let msg = if reasons.is_empty() {
                    "cedar denied".to_string()
                } else {
                    format!("cedar denied: [{}]", reasons.join(", "))
                };
                Err(ConnectError::permission_denied(msg))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use connectrpc_cedar::CedarRequest;

    // Minimal valid schema + policy so we can build a real CedarAuthorizer.
    const SCHEMA: &str = r#"
        entity User;
        entity Resource;
        action "act" appliesTo { principal: [User], resource: [Resource] };
    "#;
    const POLICY: &str = "permit(principal, action, resource);";

    // Compile-level guarantee that CedarInterceptor satisfies the
    // connectrpc::Interceptor bound (async_trait + Send + Sync + 'static).
    // Behavioural verification happens via the example worker integration,
    // since `Next` is constructed only by the dispatcher.
    #[test]
    fn implements_interceptor() {
        fn assert_interceptor<I: Interceptor>(_: &I) {}
        let authz = Arc::new(CedarAuthorizer::from_str(SCHEMA, POLICY).unwrap());
        let extractor = |_req: &UnaryRequest| -> Option<CedarRequest> { None };
        let shadow = CedarInterceptor::shadow(Arc::clone(&authz), extractor);
        let enforce = CedarInterceptor::enforce(authz, extractor).skip_paths(["/health"]);
        assert_interceptor(&shadow);
        assert_interceptor(&enforce);
    }
}
