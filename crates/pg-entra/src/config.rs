/// Multi-tenant authority segment for work/school accounts.
pub const DEFAULT_TENANT: &str = "organizations";

/// Azure CLI's public client id.
///
/// Every tenant already trusts it for `https://ossrdbms-aad.database.windows.net`
/// (that is why `az account get-access-token --resource-type oss-rdbms` works
/// without an app registration) and it is registered with an
/// `http://localhost` loopback redirect. Organisations that would rather not
/// piggy-back on it can register their own public client and set it per
/// profile.
pub const DEFAULT_CLIENT_ID: &str = "04b07795-8ddb-461a-bbee-02f9e1bf7b46";

pub const DEFAULT_AUTHORITY: &str = "https://login.microsoftonline.com";

/// Resource scope for Azure Database for PostgreSQL (flexible server and the
/// legacy single server alike).
pub const OSSRDBMS_SCOPE: &str = "https://ossrdbms-aad.database.windows.net/.default";

/// `offline_access` is what yields a refresh token; the other two make the
/// consent screen show the signed-in account.
const OIDC_SCOPES: &str = "openid profile offline_access";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntraConfig {
    /// Base URL of the identity platform. Overridable so tests can point at a
    /// local mock; production callers leave it alone.
    pub authority: String,
    pub tenant: String,
    pub client_id: String,
}

impl Default for EntraConfig {
    fn default() -> Self {
        Self {
            authority: DEFAULT_AUTHORITY.to_string(),
            tenant: DEFAULT_TENANT.to_string(),
            client_id: DEFAULT_CLIENT_ID.to_string(),
        }
    }
}

impl EntraConfig {
    /// Build from optional per-profile overrides; blank strings fall back to
    /// the defaults.
    pub fn new(tenant: Option<&str>, client_id: Option<&str>) -> Self {
        let pick = |v: Option<&str>, default: &str| {
            v.map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(default)
                .to_string()
        };
        Self {
            authority: DEFAULT_AUTHORITY.to_string(),
            tenant: pick(tenant, DEFAULT_TENANT),
            client_id: pick(client_id, DEFAULT_CLIENT_ID),
        }
    }

    pub fn authorize_endpoint(&self) -> String {
        format!(
            "{}/{}/oauth2/v2.0/authorize",
            self.authority.trim_end_matches('/'),
            self.tenant
        )
    }

    pub fn token_endpoint(&self) -> String {
        format!(
            "{}/{}/oauth2/v2.0/token",
            self.authority.trim_end_matches('/'),
            self.tenant
        )
    }

    pub fn scope(&self) -> String {
        format!("{OSSRDBMS_SCOPE} {OIDC_SCOPES}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_overrides_fall_back_to_defaults() {
        let cfg = EntraConfig::new(Some("  "), None);
        assert_eq!(cfg.tenant, DEFAULT_TENANT);
        assert_eq!(cfg.client_id, DEFAULT_CLIENT_ID);
        let cfg = EntraConfig::new(Some(" contoso.onmicrosoft.com "), Some("abc"));
        assert_eq!(cfg.tenant, "contoso.onmicrosoft.com");
        assert_eq!(cfg.client_id, "abc");
    }

    #[test]
    fn endpoints_follow_v2_layout() {
        let cfg = EntraConfig::new(Some("contoso.onmicrosoft.com"), None);
        assert_eq!(
            cfg.token_endpoint(),
            "https://login.microsoftonline.com/contoso.onmicrosoft.com/oauth2/v2.0/token"
        );
        assert!(cfg.authorize_endpoint().ends_with("/oauth2/v2.0/authorize"));
        assert!(cfg.scope().contains("offline_access"));
    }
}
