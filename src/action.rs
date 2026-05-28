//! Path → Cedar Action mapping.
//!
//! Connect-RPC paths look like `/workers.org.v1.OrgService/CreateOrganization`.
//! We map that directly to `Action::"workers.org.v1.OrgService.CreateOrganization"`
//! — same string, with the leading slash dropped and the slash before
//! the method replaced with a dot. This means a route is *automatically*
//! addressable in Cedar policies by its fully-qualified proto path.
//!
//! Why this mapping rather than a lookup table:
//! - One-to-one with the proto definition. No drift between routes and
//!   actions; if you can call it, Cedar can authorize it.
//! - No build-time codegen for a static map (which would invert in
//!   `build.rs` later — see examples/multitenant-policies/ROADMAP.md
//!   item D).
//! - Forward-compatible: adding an RPC just adds an action, no code
//!   change in this crate.

use cedar_policy::EntityUid;

/// Parse a ConnectRPC path into a Cedar action EntityUid.
///
/// Accepts both `/pkg.v1.Service/Method` and `pkg.v1.Service/Method`.
/// Returns `None` for paths that don't match the Connect shape (e.g.
/// `/healthz`, `/oauth/callback`) — those should be handled by
/// `CedarLayer::skip_paths()`, not by guessing a Cedar action.
///
/// # Examples
///
/// ```
/// use connectrpc_cedar::action::action_from_path;
///
/// let action = action_from_path("/workers.org.v1.OrgService/CreateOrganization").unwrap();
/// assert_eq!(
///     action.to_string(),
///     r#"Action::"workers.org.v1.OrgService.CreateOrganization""#
/// );
/// ```
pub fn action_from_path(path: &str) -> Option<EntityUid> {
    let trimmed = path.trim_start_matches('/');
    // Last `/` separates Service from Method.
    let (service, method) = trimmed.rsplit_once('/')?;
    if service.is_empty() || method.is_empty() {
        return None;
    }
    // Service must contain a dot (`pkg.v1.Service`) — Connect uses
    // qualified service names. This guards against accidentally
    // mapping `/foo/bar`-style REST paths.
    if !service.contains('.') {
        return None;
    }
    let id = format!("{service}.{method}");
    // Action ids may not contain `:`, `\`, or `"` per Cedar spec, and
    // our proto-path mapping never produces those — but we still feed
    // through Cedar's parser so any future Cedar tightening surfaces
    // as a parse error rather than a corrupt EntityUid.
    let euid = format!(r#"Action::"{id}""#).parse::<EntityUid>().ok()?;
    Some(euid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_connectrpc_path() {
        let action = action_from_path("/workers.org.v1.OrgService/CreateOrganization").unwrap();
        assert_eq!(
            action.to_string(),
            r#"Action::"workers.org.v1.OrgService.CreateOrganization""#
        );
    }

    #[test]
    fn maps_path_without_leading_slash() {
        let action = action_from_path("workers.auth.v1.AuthService/Login").unwrap();
        assert_eq!(
            action.to_string(),
            r#"Action::"workers.auth.v1.AuthService.Login""#
        );
    }

    #[test]
    fn rejects_non_rpc_paths() {
        assert!(action_from_path("/healthz").is_none());
        assert!(action_from_path("/oauth/callback").is_none());
        assert!(action_from_path("/").is_none());
        assert!(action_from_path("").is_none());
        // Looks like Connect shape but service has no dot — REST-ish.
        assert!(action_from_path("/foo/bar").is_none());
    }

    #[test]
    fn rejects_trailing_slash() {
        // /Service/ — empty method.
        assert!(action_from_path("/workers.org.v1.OrgService/").is_none());
    }
}
