//! Shared SSRF guard for LLM-initiated HTTP fetches (`fetch_url`, `web.run`).
//!
//! Validates scheme/host, enforces network policy, resolves DNS and rejects
//! private/loopback/link-local/metadata addresses, and returns an optional
//! DNS pin so callers can bind the HTTP client to the validated address
//! (preventing TOCTOU rebinding). Callers that follow redirects must
//! re-invoke [`validate_fetch_target`] on every new Location.

use crate::network_policy::{Decision, NetworkPolicyDecider};
use crate::tools::spec::{ToolContext, ToolError};
use std::net::IpAddr;

/// DNS pin returned when a hostname was resolved to a validated public IP.
/// Callers should pass this to `reqwest::ClientBuilder::resolve` so the
/// connection uses the pre-validated address instead of re-resolving.
pub(crate) type DnsPin = Option<(String, IpAddr)>;

/// Check if an IP address is loopback, private, link-local, cloud-metadata,
/// multicast, or reserved — all addresses that should not be reachable via
/// an LLM-initiated fetch request (SSRF prevention).
pub(crate) fn is_restricted_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_multicast()
                || v4.is_broadcast()
                || v4.is_unspecified()
                // 100.64.0.0/10 — Carrier-grade NAT (CGNAT / shared address space)
                || matches!(v4.octets(), [100, 64..=127, ..])
                // 169.254.169.254 — cloud metadata (AWS/GCP/Azure)
                || *ip == IpAddr::V4(std::net::Ipv4Addr::new(169, 254, 169, 254))
                // 198.18.0.0/15 — IETF benchmark testing
                || matches!(v4.octets(), [198, 18..=19, ..])
                // 240.0.0.0/4 — reserved (former Class E)
                || v4.octets()[0] >= 240
        }
        IpAddr::V6(v6) => {
            // IPv4-mapped IPv6 addresses (::ffff:a.b.c.d) — unwrap and check as IPv4
            // to prevent bypass via ::ffff:127.0.0.1 etc.
            if v6.is_unspecified()
                || matches!(v6.octets(), [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, ..])
            {
                return true;
            }
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_restricted_ip(&IpAddr::V4(v4));
            }
            v6.is_loopback()
                || v6.is_multicast()
                || matches!(v6.segments(), [0xfc00..=0xfdff, ..]) // ULA fc00::/7
                || matches!(v6.segments(), [0xfe80..=0xfebf, ..]) // Link-local fe80::/10
        }
    }
}

/// Validate that `url` is a safe fetch target under SSRF and network policy.
///
/// On success returns an optional DNS pin `(hostname, ip)` for hostnames that
/// were resolved; literal public IPs return `None` (no pin needed).
///
/// `tool` is the policy/audit label (e.g. `"fetch_url"`, `"web_run"`).
pub(crate) async fn validate_fetch_target(
    url: &reqwest::Url,
    context: &ToolContext,
    tool: &str,
) -> Result<DnsPin, ToolError> {
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(ToolError::invalid_input(
            "only http:// and https:// URLs are supported",
        ));
    }

    let host = url
        .host_str()
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| ToolError::invalid_input("URL must include a host"))?;

    validate_network_policy(&host, context, tool)?;

    // SSRF protection: resolve hostname and reject private/link-local/loopback IPs.
    // Prevents LLM-prompted requests to cloud metadata (169.254.169.254),
    // localhost services, and internal networks.
    if host == "localhost" || host == "localhost.localdomain" {
        return Err(ToolError::permission_denied(
            "requests to localhost are not allowed",
        ));
    }
    // Normalize bracketed IPv6 literals before the literal-IP check so they
    // route through the same restricted-IP policy as unbracketed forms
    // (GHSA-88gh-2526-gfrr).
    let ip_candidate = host
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(host.as_str());
    if let Ok(ip) = ip_candidate.parse::<IpAddr>() {
        if is_restricted_ip(&ip) {
            return Err(ToolError::permission_denied(format!(
                "IP {ip} is a restricted address (private/loopback/link-local)"
            )));
        }
        return Ok(None);
    }

    let addrs = tokio::net::lookup_host((host.as_str(), 0u16))
        .await
        .map_err(|e| {
            ToolError::permission_denied(format!(
                "could not resolve host before {tool} request: {e}"
            ))
        })?;
    let mut first_valid: Option<IpAddr> = None;
    for addr in addrs {
        validate_dns_resolved_ip(&host, &addr.ip(), context.network_policy.as_ref(), tool)?;
        if first_valid.is_none() {
            first_valid = Some(addr.ip());
        }
    }

    let Some(validated_ip) = first_valid else {
        return Err(ToolError::permission_denied(format!(
            "host resolved to no addresses before {tool} request"
        )));
    };
    Ok(Some((host, validated_ip)))
}

pub(crate) fn validate_network_policy(
    host: &str,
    context: &ToolContext,
    tool: &str,
) -> Result<(), ToolError> {
    let Some(decider) = context.network_policy.as_ref() else {
        return Ok(());
    };

    match decider.evaluate(host, tool) {
        Decision::Allow => Ok(()),
        Decision::Deny => Err(ToolError::permission_denied(format!(
            "network call to '{host}' blocked by network policy"
        ))),
        Decision::Prompt => Err(ToolError::permission_denied(format!(
            "network call to '{host}' requires approval; \
             re-run after `/network allow {host}` or set network.default = \"allow\" in config"
        ))),
    }
}

pub(crate) fn validate_dns_resolved_ip(
    host: &str,
    ip: &IpAddr,
    decider: Option<&NetworkPolicyDecider>,
    tool: &str,
) -> Result<(), ToolError> {
    if !is_restricted_ip(ip) {
        return Ok(());
    }

    // Allow the resolved IP past the restricted-IP block if either:
    //   * it falls inside a configured fake-IP placeholder range (a TUN /
    //     transparent-proxy setup in `fake-ip` mode resolves every host into a
    //     reserved range such as `198.18.0.0/15`), or
    //   * the host is on the explicitly-trusted proxy list.
    // Real private/loopback/link-local/metadata IPs match neither and stay blocked.
    if let Some(decider) = decider
        && (decider.is_trusted_fakeip_addr(ip) || decider.trusts_proxy_fakeip_host(host))
    {
        decider.record_trusted_proxy_fakeip_allow(host, tool);
        return Ok(());
    }

    Err(ToolError::permission_denied(format!(
        "resolved IP {ip} is a restricted address (private/loopback/link-local)"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::spec::ToolContext;
    use std::path::PathBuf;

    fn ctx() -> ToolContext {
        ToolContext::new(PathBuf::from("."))
    }

    #[test]
    fn rejects_private_localhost_literal() {
        assert!(is_restricted_ip(&"127.0.0.1".parse().unwrap()));
        assert!(is_restricted_ip(&"::1".parse().unwrap()));
    }

    #[test]
    fn rejects_private_rfc1918() {
        assert!(is_restricted_ip(&"10.0.0.1".parse().unwrap()));
        assert!(is_restricted_ip(&"172.16.0.1".parse().unwrap()));
        assert!(is_restricted_ip(&"192.168.1.1".parse().unwrap()));
    }

    #[test]
    fn rejects_cloud_metadata() {
        assert!(is_restricted_ip(&"169.254.169.254".parse().unwrap()));
    }

    #[test]
    fn rejects_link_local() {
        assert!(is_restricted_ip(&"169.254.1.1".parse().unwrap()));
    }

    #[test]
    fn rejects_cgnat() {
        assert!(is_restricted_ip(&"100.64.0.1".parse().unwrap()));
        assert!(!is_restricted_ip(&"100.63.0.1".parse().unwrap()));
        assert!(!is_restricted_ip(&"100.128.0.1".parse().unwrap()));
    }

    #[test]
    fn rejects_ipv6_ula() {
        assert!(is_restricted_ip(&"fc00::1".parse().unwrap()));
        assert!(is_restricted_ip(&"fd12:3456::1".parse().unwrap()));
    }

    #[test]
    fn rejects_ipv4_mapped_ipv6() {
        // ::ffff:127.0.0.1 — IPv4-mapped IPv6 loopback bypass
        assert!(is_restricted_ip(&"::ffff:127.0.0.1".parse().unwrap()));
        assert!(is_restricted_ip(&"::ffff:10.0.0.1".parse().unwrap()));
        assert!(is_restricted_ip(&"::ffff:169.254.169.254".parse().unwrap()));
        assert!(is_restricted_ip(&"::ffff:192.168.1.1".parse().unwrap()));
        // :: (unspecified)
        assert!(is_restricted_ip(&"::".parse().unwrap()));
    }

    #[test]
    fn allows_public_ips() {
        assert!(!is_restricted_ip(&"8.8.8.8".parse().unwrap()));
        assert!(!is_restricted_ip(&"1.1.1.1".parse().unwrap()));
        assert!(!is_restricted_ip(&"93.184.216.34".parse().unwrap()));
        assert!(!is_restricted_ip(&"2606:4700::1".parse().unwrap()));
    }

    #[tokio::test]
    async fn redirected_localhost_hostname_is_rejected() {
        let url = reqwest::Url::parse("http://localhost:8080/admin").unwrap();
        let err = validate_fetch_target(&url, &ctx(), "fetch_url")
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("localhost"));
    }

    #[tokio::test]
    async fn redirected_private_ip_literal_is_rejected() {
        let url = reqwest::Url::parse("http://169.254.169.254/latest/meta-data").unwrap();
        let err = validate_fetch_target(&url, &ctx(), "fetch_url")
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("restricted address"));
    }

    // GHSA-88gh-2526-gfrr — regression coverage for bracketed IPv6 literals.
    #[tokio::test]
    async fn rejects_ipv6_literal_loopback() {
        let url = reqwest::Url::parse("http://[::1]/").unwrap();
        let err = validate_fetch_target(&url, &ctx(), "fetch_url")
            .await
            .expect_err("[::1] must be rejected as restricted");
        assert!(format!("{err}").contains("restricted"));
    }

    #[tokio::test]
    async fn rejects_ipv6_literal_ula() {
        let url = reqwest::Url::parse("http://[fc00::1]/").unwrap();
        let err = validate_fetch_target(&url, &ctx(), "fetch_url")
            .await
            .expect_err("[fc00::1] must be rejected as restricted");
        assert!(format!("{err}").contains("restricted"));
    }

    #[tokio::test]
    async fn rejects_ipv6_literal_link_local() {
        let url = reqwest::Url::parse("http://[fe80::1]/").unwrap();
        let err = validate_fetch_target(&url, &ctx(), "fetch_url")
            .await
            .expect_err("[fe80::1] must be rejected as restricted");
        assert!(format!("{err}").contains("restricted"));
    }

    #[tokio::test]
    async fn rejects_ipv6_literal_ipv4_mapped_loopback() {
        let url = reqwest::Url::parse("http://[::ffff:127.0.0.1]/").unwrap();
        let err = validate_fetch_target(&url, &ctx(), "fetch_url")
            .await
            .expect_err("[::ffff:127.0.0.1] must be rejected as restricted");
        assert!(format!("{err}").contains("restricted"));
    }

    #[tokio::test]
    async fn rejects_ipv6_literal_unspecified() {
        let url = reqwest::Url::parse("http://[::]/").unwrap();
        let err = validate_fetch_target(&url, &ctx(), "fetch_url")
            .await
            .expect_err("[::] must be rejected as restricted");
        assert!(format!("{err}").contains("restricted"));
    }

    #[tokio::test]
    async fn redirected_host_respects_network_policy() {
        use crate::network_policy::{Decision, NetworkPolicy, NetworkPolicyDecider};
        let policy = NetworkPolicy {
            default: Decision::Deny.into(),
            allow: vec!["api.deepseek.com".to_string()],
            deny: vec![],
            proxy: Vec::new(),
            audit: false,
        };
        let decider = NetworkPolicyDecider::new(policy, None);
        let ctx = ToolContext::new(PathBuf::from(".")).with_network_policy(decider);
        let url = reqwest::Url::parse("https://example.com/redirect-target").unwrap();
        let err = validate_fetch_target(&url, &ctx, "fetch_url")
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("blocked"));
    }

    #[tokio::test]
    async fn unresolved_hostname_is_rejected_before_request() {
        let url =
            reqwest::Url::parse("https://codewhale-unresolvable-fetch-target.invalid/resource")
                .unwrap();
        let err = validate_fetch_target(&url, &ctx(), "fetch_url")
            .await
            .expect_err("unresolved host must fail preflight");
        let message = format!("{err}");
        assert!(
            message.contains("could not resolve host") || message.contains("restricted address"),
            "error must identify preflight DNS or restricted-IP failure; got {err}"
        );
    }

    #[test]
    fn restricted_dns_result_is_denied_without_proxy_opt_in() {
        let ip = "198.18.0.1".parse().unwrap();

        let err = validate_dns_resolved_ip("github.com", &ip, None, "fetch_url")
            .expect_err("fake-IP DNS result must be denied by default");

        assert!(format!("{err}").contains("resolved IP 198.18.0.1 is a restricted address"));
    }

    #[test]
    fn proxy_opt_in_allows_restricted_dns_for_matching_host() {
        use crate::network_policy::{Decision, NetworkPolicy, NetworkPolicyDecider};

        let policy = NetworkPolicy {
            default: Decision::Allow.into(),
            allow: Vec::new(),
            deny: Vec::new(),
            proxy: vec!["github.com".to_string()],
            audit: false,
        };
        let decider = NetworkPolicyDecider::new(policy, None);
        let ip = "198.18.0.1".parse().unwrap();

        validate_dns_resolved_ip("github.com", &ip, Some(&decider), "fetch_url")
            .expect("proxy opt-in should allow fake-IP DNS for matching host");
    }

    #[test]
    fn proxy_opt_in_does_not_allow_unlisted_host() {
        use crate::network_policy::{Decision, NetworkPolicy, NetworkPolicyDecider};

        let policy = NetworkPolicy {
            default: Decision::Allow.into(),
            allow: Vec::new(),
            deny: Vec::new(),
            proxy: vec!["github.com".to_string()],
            audit: false,
        };
        let decider = NetworkPolicyDecider::new(policy, None);
        let ip = "198.18.0.1".parse().unwrap();

        let err = validate_dns_resolved_ip("example.com", &ip, Some(&decider), "fetch_url")
            .expect_err("proxy opt-in must be scoped to configured hosts");

        assert!(format!("{err}").contains("resolved IP 198.18.0.1 is a restricted address"));
    }

    #[test]
    fn proxy_dns_allow_is_audited() {
        use crate::network_policy::{
            Decision, NetworkAuditor, NetworkPolicy, NetworkPolicyDecider,
        };
        use tempfile::tempdir;

        let dir = tempdir().expect("tempdir");
        let auditor = NetworkAuditor::new(dir.path().join("audit.log"), true);
        let policy = NetworkPolicy {
            default: Decision::Allow.into(),
            allow: Vec::new(),
            deny: Vec::new(),
            proxy: vec!["github.com".to_string()],
            audit: true,
        };
        let decider = NetworkPolicyDecider::new(policy, Some(auditor));
        let ip = "198.18.0.1".parse().unwrap();

        validate_dns_resolved_ip("github.com", &ip, Some(&decider), "fetch_url")
            .expect("proxy DNS allow");

        let body = std::fs::read_to_string(dir.path().join("audit.log")).expect("audit log");
        assert!(body.contains("github.com"));
        assert!(body.contains("TrustedProxyFakeIp-Allow"));
    }

    #[tokio::test]
    async fn web_run_tool_label_is_used_in_dns_error() {
        let url =
            reqwest::Url::parse("https://codewhale-unresolvable-web-run-target.invalid/resource")
                .unwrap();
        let err = validate_fetch_target(&url, &ctx(), "web_run")
            .await
            .expect_err("unresolved host must fail preflight");
        let message = format!("{err}");
        // Either DNS failure (mentions web_run) or a restricted resolution.
        assert!(
            message.contains("web_run") || message.contains("restricted address"),
            "error should be labeled for web_run or report restricted IP; got {err}"
        );
    }
}
