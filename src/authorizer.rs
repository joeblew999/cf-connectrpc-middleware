//! Cedar authorizer: holds the loaded schema + policy set + (optionally)
//! a pre-populated entity store. Built once at worker boot, shared
//! across all requests via `Arc`.
//!
//! Cedar's `Schema`, `PolicySet`, and `Authorizer` are all immutable
//! once constructed and cheap to `Clone` (internally `Arc`-backed in
//! `cedar-policy` 4.x), so wrapping them in our own `Arc` is a thin
//! convenience for the layer's `Clone` derive.

use std::sync::Arc;

use cedar_policy::{
    Authorizer, Context, Decision, Entities, EntityUid, PolicySet, Request, Schema,
};

/// Errors raised when constructing a [`CedarAuthorizer`]. Parse-time
/// errors only — runtime evaluation errors come through
/// `Decision::Deny` + diagnostics, not as errors.
#[derive(Debug)]
pub enum CedarAuthorizerError {
    Schema(String),
    Policies(String),
    Validation(String),
    Entities(String),
}

impl std::fmt::Display for CedarAuthorizerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Schema(e) => write!(f, "schema parse error: {e}"),
            Self::Policies(e) => write!(f, "policy parse error: {e}"),
            Self::Validation(e) => write!(f, "policy validation against schema failed: {e}"),
            Self::Entities(e) => write!(f, "entities parse error: {e}"),
        }
    }
}

impl std::error::Error for CedarAuthorizerError {}

/// Constructed Cedar authorizer. Cheap to clone; the layer holds one
/// per worker instance.
#[derive(Clone)]
pub struct CedarAuthorizer {
    inner: Arc<Inner>,
}

struct Inner {
    schema: Schema,
    policies: PolicySet,
    entities: Entities,
    authorizer: Authorizer,
}

impl CedarAuthorizer {
    /// Construct from a Cedar schema string + concatenated policy text.
    /// Policies are validated against the schema; mismatches fail at
    /// construction time, not at request time.
    ///
    /// The entity store is empty. Most uses don't need a populated
    /// store — relations come through the per-request Cedar context.
    /// For setups that pre-load entities, use [`Self::with_entities`].
    pub fn from_str(schema: &str, policies: &str) -> Result<Self, CedarAuthorizerError> {
        let (schema, _warnings) =
            Schema::from_cedarschema_str(schema).map_err(|e| CedarAuthorizerError::Schema(e.to_string()))?;
        let policies = policies
            .parse::<PolicySet>()
            .map_err(|e| CedarAuthorizerError::Policies(e.to_string()))?;
        // Validate policies against schema up-front. This is the same
        // check `cedar validate` runs at build time; doing it again at
        // worker boot guarantees a misconfigured deploy fails to start
        // rather than silently approving every request.
        let validator = cedar_policy::Validator::new(schema.clone());
        let result = validator.validate(&policies, cedar_policy::ValidationMode::default());
        if !result.validation_passed() {
            return Err(CedarAuthorizerError::Validation(format!("{result}")));
        }
        // Pass the schema to Entities::from_entities so the schema's
        // declared action-group memberships (`action X in [GroupY]`)
        // populate the entity hierarchy. Without this, policies that
        // check `action in Action::"GroupY"` default-deny because the
        // membership relation isn't visible at request time.
        let entities = Entities::from_entities(std::iter::empty(), Some(&schema))
            .map_err(|e| CedarAuthorizerError::Entities(e.to_string()))?;
        Ok(Self {
            inner: Arc::new(Inner {
                schema,
                policies,
                entities,
                authorizer: Authorizer::new(),
            }),
        })
    }

    /// Same as [`Self::from_str`] but loads an entity store too. Useful
    /// for relations that don't change per-request (e.g. a fixed list
    /// of admin user UIDs); per-request relations should still come
    /// through `context.*_relations`.
    pub fn with_entities(
        schema: &str,
        policies: &str,
        entities_json: &str,
    ) -> Result<Self, CedarAuthorizerError> {
        let base = Self::from_str(schema, policies)?;
        let entities = Entities::from_json_str(entities_json, Some(&base.inner.schema))
            .map_err(|e| CedarAuthorizerError::Entities(e.to_string()))?;
        Ok(Self {
            inner: Arc::new(Inner {
                schema: base.inner.schema.clone(),
                policies: base.inner.policies.clone(),
                entities,
                authorizer: Authorizer::new(),
            }),
        })
    }

    /// Evaluate a request. Returns `(Decision, reasons)` where `reasons`
    /// is the list of policy ids that contributed (empty for default-deny).
    /// The layer uses both — reasons go into shadow-mode logs.
    pub fn is_authorized(
        &self,
        principal: &EntityUid,
        action: &EntityUid,
        resource: &EntityUid,
        context: Context,
    ) -> (Decision, Vec<String>) {
        let req = Request::new(
            principal.clone(),
            action.clone(),
            resource.clone(),
            context,
            Some(&self.inner.schema),
        );
        let req = match req {
            Ok(r) => r,
            Err(e) => {
                // Schema-invalid request (e.g. action not declared in
                // schema for this principal/resource pair). Cedar
                // can't evaluate it; default-deny is the safe answer.
                return (Decision::Deny, vec![format!("schema-error: {e}")]);
            }
        };
        let response = self
            .inner
            .authorizer
            .is_authorized(&req, &self.inner.policies, &self.inner.entities);
        let reasons = response
            .diagnostics()
            .reason()
            .map(|id| id.to_string())
            .collect();
        (response.decision(), reasons)
    }
}

