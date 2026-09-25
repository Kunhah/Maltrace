//! Canonical entity ontology.
//!
//! SpiderFoot emits 160+ distinct event types — far too noisy to surface
//! directly on a graph canvas. Here we collapse them into ~22 canonical
//! entity types (`Domain`, `EmailAddress`, `Person`, …) that an investigator
//! actually thinks in terms of. The raw SpiderFoot event type is preserved
//! on each entity (in `source_event_type`) so nothing is lost, and so we can
//! present "this Domain was emitted as an AFFILIATE_INTERNET_NAME" in the
//! inspector if a user cares.
//!
//! When SpiderFoot grows new event types, unknown ones land in `Generic` and
//! show up as such on the canvas — the app still works; mapping coverage
//! just degrades gracefully.

use serde::Serialize;

/// A canonical, human-readable entity type. Stored as a string id so it
/// round-trips through SQLite and JSON cleanly.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct CanonicalType {
    /// Stable id (used as the DB `entity_type` value). PascalCase.
    pub id: &'static str,
    /// What we show in the UI badge.
    pub label: &'static str,
    /// Coarse classification — drives color and grouping.
    pub kind: Kind,
    /// Hex color for the node — kept on the backend so adding a type doesn't
    /// require also editing CSS.
    pub color: &'static str,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Who: people and their handles.
    Identity,
    /// What: domains, IPs, URLs, files — first-class objects.
    Asset,
    /// Records and metadata about other entities.
    Evidence,
    /// Risk signals: vulnerabilities, blacklists, breaches.
    Threat,
    /// Catch-all for unmapped event types.
    Generic,
}

/// All canonical types. Kept in a single const array so the frontend can
/// enumerate via a Tauri command and so adding a type is one edit.
pub const ALL: &[CanonicalType] = &[
    // ─── Identity ──────────────────────────────────────────────
    CanonicalType { id: "Person",          label: "Person",          kind: Kind::Identity,   color: "#f472b6" },
    CanonicalType { id: "EmailAddress",    label: "Email",           kind: Kind::Identity,   color: "#a855f7" },
    CanonicalType { id: "PhoneNumber",     label: "Phone",           kind: Kind::Identity,   color: "#c084fc" },
    CanonicalType { id: "Username",        label: "Username",        kind: Kind::Identity,   color: "#e879f9" },
    CanonicalType { id: "Organization",    label: "Organization",    kind: Kind::Identity,   color: "#fb7185" },

    // ─── Asset (network) ───────────────────────────────────────
    CanonicalType { id: "Domain",          label: "Domain",          kind: Kind::Asset,      color: "#22c55e" },
    CanonicalType { id: "IpAddress",       label: "IP",              kind: Kind::Asset,      color: "#f59e0b" },
    CanonicalType { id: "Netblock",        label: "Netblock",        kind: Kind::Asset,      color: "#fbbf24" },
    CanonicalType { id: "Asn",             label: "ASN",             kind: Kind::Asset,      color: "#facc15" },
    CanonicalType { id: "Url",             label: "URL",             kind: Kind::Asset,      color: "#34d399" },
    CanonicalType { id: "NetworkService",  label: "Service",         kind: Kind::Asset,      color: "#10b981" },
    CanonicalType { id: "CloudResource",   label: "Cloud Resource",  kind: Kind::Asset,      color: "#06b6d4" },
    CanonicalType { id: "Provider",        label: "Provider",        kind: Kind::Asset,      color: "#0ea5e9" },
    CanonicalType { id: "SslCertificate",  label: "TLS Cert",        kind: Kind::Asset,      color: "#38bdf8" },
    CanonicalType { id: "CryptoAddress",   label: "Crypto",          kind: Kind::Asset,      color: "#fb923c" },
    CanonicalType { id: "File",            label: "File",            kind: Kind::Asset,      color: "#94a3b8" },
    CanonicalType { id: "Image",           label: "Image",           kind: Kind::Asset,      color: "#fde047" },
    CanonicalType { id: "Face",            label: "Face",            kind: Kind::Identity,   color: "#fda4af" },
    CanonicalType { id: "Software",        label: "Software",        kind: Kind::Asset,      color: "#a3e635" },

    // ─── Evidence ──────────────────────────────────────────────
    CanonicalType { id: "WhoisRecord",     label: "WHOIS",           kind: Kind::Evidence,   color: "#64748b" },
    CanonicalType { id: "DnsRecord",       label: "DNS Record",      kind: Kind::Evidence,   color: "#6b7280" },
    CanonicalType { id: "Geolocation",     label: "Location",        kind: Kind::Evidence,   color: "#7dd3fc" },
    CanonicalType { id: "Hash",            label: "Hash / Secret",   kind: Kind::Evidence,   color: "#fde68a" },
    CanonicalType { id: "PaymentInfo",     label: "Payment Info",    kind: Kind::Evidence,   color: "#fcd34d" },
    CanonicalType { id: "WebMetadata",     label: "Web Metadata",    kind: Kind::Evidence,   color: "#9ca3af" },

    // ─── Threat ────────────────────────────────────────────────
    CanonicalType { id: "Vulnerability",   label: "Vulnerability",   kind: Kind::Threat,     color: "#ef4444" },
    CanonicalType { id: "ThreatIndicator", label: "Threat",          kind: Kind::Threat,     color: "#dc2626" },

    // ─── Collection (synthetic — a bag of N entities of one type) ─
    // Created by `run_transform` when a single invocation returns more
    // entities of one canonical type than COLLECTION_THRESHOLD. The
    // children are stored in `entity.data` and materialised on expand.
    CanonicalType { id: "Collection",      label: "Collection",      kind: Kind::Generic,    color: "#94a3b8" },

    // ─── Fallback ──────────────────────────────────────────────
    CanonicalType { id: "Generic",         label: "Generic",         kind: Kind::Generic,    color: "#64748b" },
];

/// Threshold above which a single transform run's outputs of one canonical
/// type are collapsed into a Collection entity instead of N individual
/// entities. Picked so a domain that resolves to ~20 IPs becomes a
/// browseable Collection rather than a hairball of nodes; small batches
/// (DNS A+AAAA+MX) still appear inline.
pub const COLLECTION_THRESHOLD: usize = 10;

/// Source-of-truth mapping table: (SpiderFoot event type, canonical id).
/// Both `canonical_for` and `sf_inputs_for` derive from this so the two
/// directions of the mapping can never drift.
///
/// The default SF event for a canonical type is the FIRST entry whose
/// canonical matches — keep "the most representative" SF event first per
/// group (e.g. DOMAIN_NAME before AFFILIATE_DOMAIN_NAME).
const MAPPINGS: &[(&str, &str)] = &[
    // ─── Domain ───────────────────────────────────────────────
    ("DOMAIN_NAME",                       "Domain"),
    ("INTERNET_NAME",                     "Domain"),
    ("DOMAIN_NAME_PARENT",                "Domain"),
    ("INTERNET_NAME_UNRESOLVED",          "Domain"),
    ("AFFILIATE_DOMAIN_NAME",             "Domain"),
    ("AFFILIATE_DOMAIN_UNREGISTERED",     "Domain"),
    ("AFFILIATE_INTERNET_NAME",           "Domain"),
    ("AFFILIATE_INTERNET_NAME_HIJACKABLE","Domain"),
    ("AFFILIATE_INTERNET_NAME_UNRESOLVED","Domain"),
    ("CO_HOSTED_SITE",                    "Domain"),
    ("CO_HOSTED_SITE_DOMAIN",             "Domain"),
    ("SIMILARDOMAIN",                     "Domain"),

    // ─── IpAddress ────────────────────────────────────────────
    ("IP_ADDRESS",                "IpAddress"),
    ("IPV6_ADDRESS",              "IpAddress"),
    ("AFFILIATE_IPADDR",          "IpAddress"),
    ("AFFILIATE_IPV6_ADDRESS",    "IpAddress"),
    ("INTERNAL_IP_ADDRESS",       "IpAddress"),

    // ─── Netblock / Asn ───────────────────────────────────────
    ("NETBLOCK_OWNER",     "Netblock"),
    ("NETBLOCK_MEMBER",    "Netblock"),
    ("NETBLOCKV6_OWNER",   "Netblock"),
    ("NETBLOCKV6_MEMBER",  "Netblock"),
    ("BGP_AS_OWNER",       "Asn"),
    ("BGP_AS_MEMBER",      "Asn"),

    // ─── Identity ─────────────────────────────────────────────
    ("EMAILADDR",                "EmailAddress"),
    ("AFFILIATE_EMAILADDR",      "EmailAddress"),
    ("EMAILADDR_COMPROMISED",    "EmailAddress"),
    ("EMAILADDR_DELIVERABLE",    "EmailAddress"),
    ("EMAILADDR_UNDELIVERABLE",  "EmailAddress"),
    ("EMAILADDR_DISPOSABLE",     "EmailAddress"),
    ("EMAILADDR_GENERIC",        "EmailAddress"),
    ("PHONE_NUMBER",             "PhoneNumber"),
    ("PHONE_NUMBER_COMPROMISED", "PhoneNumber"),
    ("PHONE_NUMBER_TYPE",        "PhoneNumber"),
    ("HUMAN_NAME",               "Person"),
    ("JOB_TITLE",                "Person"),
    ("USERNAME",                 "Username"),
    ("ACCOUNT_EXTERNAL_OWNED",   "Username"),
    ("SIMILAR_ACCOUNT_EXTERNAL", "Username"),
    ("SOCIAL_MEDIA",             "Username"),
    ("COMPANY_NAME",             "Organization"),
    ("AFFILIATE_COMPANY_NAME",   "Organization"),
    ("LEI",                      "Organization"),

    // ─── Url / File / Image ───────────────────────────────────
    ("LINKED_URL_INTERNAL",        "Url"),
    ("LINKED_URL_EXTERNAL",        "Url"),
    ("URL_ADBLOCKED_INTERNAL",     "Url"),
    ("URL_ADBLOCKED_EXTERNAL",     "Url"),
    ("URL_FLASH",                  "Url"),
    ("URL_FLASH_HISTORIC",         "Url"),
    ("URL_FORM",                   "Url"),
    ("URL_FORM_HISTORIC",          "Url"),
    ("URL_JAVASCRIPT",             "Url"),
    ("URL_JAVASCRIPT_HISTORIC",    "Url"),
    ("URL_JAVA_APPLET",            "Url"),
    ("URL_JAVA_APPLET_HISTORIC",   "Url"),
    ("URL_PASSWORD",               "Url"),
    ("URL_PASSWORD_HISTORIC",      "Url"),
    ("URL_STATIC",                 "Url"),
    ("URL_STATIC_HISTORIC",        "Url"),
    ("URL_UPLOAD",                 "Url"),
    ("URL_UPLOAD_HISTORIC",        "Url"),
    ("URL_WEB_FRAMEWORK",          "Url"),
    ("URL_WEB_FRAMEWORK_HISTORIC", "Url"),
    ("DARKNET_MENTION_URL",        "Url"),
    ("LEAKSITE_URL",               "Url"),
    ("INTERESTING_FILE",           "File"),
    ("INTERESTING_FILE_HISTORIC",  "File"),
    ("JUNK_FILE",                  "File"),
    ("RAW_FILE_META_DATA",         "File"),
    // The face-search sidecar emits these (legacy SpiderFoot has no Image
    // or Face events).
    ("IMAGE",                      "Image"),
    ("FACE",                       "Face"),

    // ─── Infrastructure ───────────────────────────────────────
    ("TCP_PORT_OPEN",          "NetworkService"),
    ("TCP_PORT_OPEN_BANNER",   "NetworkService"),
    ("UDP_PORT_OPEN",          "NetworkService"),
    ("UDP_PORT_OPEN_INFO",     "NetworkService"),
    ("WEBSERVER_BANNER",       "NetworkService"),
    ("WEBSERVER_HTTPHEADERS",  "NetworkService"),
    ("WEBSERVER_STRANGEHEADER","NetworkService"),
    ("WEBSERVER_TECHNOLOGY",   "NetworkService"),
    ("WIFI_ACCESS_POINT",      "NetworkService"),
    ("CLOUD_STORAGE_BUCKET",       "CloudResource"),
    ("CLOUD_STORAGE_BUCKET_OPEN",  "CloudResource"),
    ("PROVIDER_DNS",        "Provider"),
    ("PROVIDER_HOSTING",    "Provider"),
    ("PROVIDER_JAVASCRIPT", "Provider"),
    ("PROVIDER_MAIL",       "Provider"),
    ("PROVIDER_TELCO",      "Provider"),
    ("TOR_EXIT_NODE",       "Provider"),
    ("VPN_HOST",            "Provider"),
    ("PROXY_HOST",          "Provider"),
    ("SSL_CERTIFICATE_RAW",       "SslCertificate"),
    ("SSL_CERTIFICATE_ISSUED",    "SslCertificate"),
    ("SSL_CERTIFICATE_ISSUER",    "SslCertificate"),
    ("SSL_CERTIFICATE_EXPIRED",   "SslCertificate"),
    ("SSL_CERTIFICATE_EXPIRING",  "SslCertificate"),
    ("SSL_CERTIFICATE_MISMATCH",  "SslCertificate"),
    ("BITCOIN_ADDRESS",   "CryptoAddress"),
    ("BITCOIN_BALANCE",   "CryptoAddress"),
    ("ETHEREUM_ADDRESS",  "CryptoAddress"),
    ("ETHEREUM_BALANCE",  "CryptoAddress"),
    ("SOFTWARE_USED",     "Software"),
    ("OPERATING_SYSTEM",  "Software"),
    ("DEVICE_TYPE",       "Software"),

    // ─── Evidence ─────────────────────────────────────────────
    ("DOMAIN_WHOIS",                   "WhoisRecord"),
    ("NETBLOCK_WHOIS",                 "WhoisRecord"),
    ("AFFILIATE_DOMAIN_WHOIS",         "WhoisRecord"),
    ("SIMILARDOMAIN_WHOIS",            "WhoisRecord"),
    ("CO_HOSTED_SITE_DOMAIN_WHOIS",    "WhoisRecord"),
    ("DOMAIN_REGISTRAR",               "WhoisRecord"),
    ("DNS_SPF",          "DnsRecord"),
    ("DNS_TEXT",         "DnsRecord"),
    ("RAW_DNS_RECORDS",  "DnsRecord"),
    ("GEOINFO",              "Geolocation"),
    ("COUNTRY_NAME",         "Geolocation"),
    ("PHYSICAL_ADDRESS",     "Geolocation"),
    ("PHYSICAL_COORDINATES", "Geolocation"),
    ("HASH",                "Hash"),
    ("HASH_COMPROMISED",    "Hash"),
    ("PASSWORD_COMPROMISED","Hash"),
    ("PGP_KEY",             "Hash"),
    ("CREDIT_CARD_NUMBER",  "PaymentInfo"),
    ("IBAN_NUMBER",         "PaymentInfo"),
    ("TARGET_WEB_CONTENT",              "WebMetadata"),
    ("TARGET_WEB_CONTENT_TYPE",         "WebMetadata"),
    ("TARGET_WEB_COOKIE",               "WebMetadata"),
    ("AFFILIATE_WEB_CONTENT",           "WebMetadata"),
    ("WEB_ANALYTICS_ID",                "WebMetadata"),
    ("DESCRIPTION_ABSTRACT",            "WebMetadata"),
    ("DESCRIPTION_CATEGORY",            "WebMetadata"),
    ("AFFILIATE_DESCRIPTION_ABSTRACT",  "WebMetadata"),
    ("AFFILIATE_DESCRIPTION_CATEGORY",  "WebMetadata"),
    ("BASE64_DATA",                     "WebMetadata"),
    ("HTTP_CODE",                       "WebMetadata"),
    ("APPSTORE_ENTRY",                  "WebMetadata"),
    ("PUBLIC_CODE_REPO",                "WebMetadata"),
    ("DARKNET_MENTION_CONTENT",         "WebMetadata"),
    ("LEAKSITE_CONTENT",                "WebMetadata"),
    ("WIKIPEDIA_PAGE_EDIT",             "WebMetadata"),
    ("RAW_RIR_DATA",                    "WebMetadata"),

    // ─── Threat ───────────────────────────────────────────────
    ("VULNERABILITY_CVE_CRITICAL", "Vulnerability"),
    ("VULNERABILITY_CVE_HIGH",     "Vulnerability"),
    ("VULNERABILITY_CVE_MEDIUM",   "Vulnerability"),
    ("VULNERABILITY_CVE_LOW",      "Vulnerability"),
    ("VULNERABILITY_DISCLOSURE",   "Vulnerability"),
    ("VULNERABILITY_GENERAL",      "Vulnerability"),
    ("MALICIOUS_ASN",                       "ThreatIndicator"),
    ("MALICIOUS_BITCOIN_ADDRESS",           "ThreatIndicator"),
    ("MALICIOUS_COHOST",                    "ThreatIndicator"),
    ("MALICIOUS_EMAILADDR",                 "ThreatIndicator"),
    ("MALICIOUS_INTERNET_NAME",             "ThreatIndicator"),
    ("MALICIOUS_IPADDR",                    "ThreatIndicator"),
    ("MALICIOUS_AFFILIATE_INTERNET_NAME",   "ThreatIndicator"),
    ("MALICIOUS_AFFILIATE_IPADDR",          "ThreatIndicator"),
    ("MALICIOUS_NETBLOCK",                  "ThreatIndicator"),
    ("MALICIOUS_PHONE_NUMBER",              "ThreatIndicator"),
    ("MALICIOUS_SUBNET",                    "ThreatIndicator"),
    ("BLACKLISTED_INTERNET_NAME",           "ThreatIndicator"),
    ("BLACKLISTED_IPADDR",                  "ThreatIndicator"),
    ("BLACKLISTED_NETBLOCK",                "ThreatIndicator"),
    ("BLACKLISTED_SUBNET",                  "ThreatIndicator"),
    ("BLACKLISTED_COHOST",                  "ThreatIndicator"),
    ("BLACKLISTED_AFFILIATE_INTERNET_NAME", "ThreatIndicator"),
    ("BLACKLISTED_AFFILIATE_IPADDR",        "ThreatIndicator"),
    ("DEFACED_INTERNET_NAME",               "ThreatIndicator"),
    ("DEFACED_IPADDR",                      "ThreatIndicator"),
    ("DEFACED_COHOST",                      "ThreatIndicator"),
    ("DEFACED_AFFILIATE_INTERNET_NAME",     "ThreatIndicator"),
    ("DEFACED_AFFILIATE_IPADDR",            "ThreatIndicator"),
];

/// Map a SpiderFoot event type to its canonical entity type id.
///
/// Returns `"Generic"` for anything not in the table — preferable to
/// returning `Option` because every entity needs *some* type, and falling
/// back keeps the app working when SpiderFoot adds new events.
pub fn canonical_for(sf_event: &str) -> &'static str {
    MAPPINGS
        .iter()
        .find(|(sf, _)| *sf == sf_event)
        .map(|(_, canonical)| *canonical)
        .unwrap_or("Generic")
}

/// Every SpiderFoot event type that maps to this canonical id, in
/// declaration order. Empty for canonical ids with no SF coverage (e.g.
/// "Generic" itself).
pub fn sf_inputs_for(canonical_id: &str) -> Vec<&'static str> {
    MAPPINGS
        .iter()
        .filter(|(_, c)| *c == canonical_id)
        .map(|(sf, _)| *sf)
        .collect()
}

/// Pick a sensible SpiderFoot event type to use when invoking a transform
/// on a manually-added entity whose `source_event_type` is null. Returns
/// the first SF event from the mapping table for that canonical, or the
/// canonical id itself as a fallback (which will almost certainly fail at
/// the sidecar — surfaced cleanly via an error).
pub fn default_sf_event_for(canonical_id: &str) -> String {
    sf_inputs_for(canonical_id)
        .first()
        .map(|s| s.to_string())
        .unwrap_or_else(|| canonical_id.to_string())
}

/// Pick the SpiderFoot event type to feed a specific transform when running
/// it against an entity.
///
/// A single canonical type maps to several SF events — e.g. a Domain is
/// both `DOMAIN_NAME` and `INTERNET_NAME`. The right-click menu marks a
/// transform compatible if it consumes *any* of them, but the backend can
/// only send one event type per run. Sending the canonical default
/// (`DOMAIN_NAME`) to a module that only consumes `INTERNET_NAME` (e.g.
/// `sfp_dnsresolve`) silently produces zero events — the classic
/// "compatible, but no data" trap.
///
/// So: prefer an event type the transform actually `consumes`. Precedence
/// matches the compatibility hook —
///   1. a concrete `source_event_type` (if the entity has one) wins when the
///      transform consumes it,
///   2. otherwise the first of the canonical's SF inputs that the transform
///      consumes,
///   3. falling back to the canonical default (preserves prior behavior and
///      surfaces a clean sidecar error if nothing matches).
///
/// `consumes` is the transform's accepted event list; an empty slice or one
/// containing `"*"` (SpiderFoot's "any" wildcard) means "no preference", so
/// we keep the most specific candidate.
pub fn input_event_for_transform(
    canonical_id: &str,
    source_event_type: Option<&str>,
    consumes: &[String],
) -> String {
    let accepts = |ev: &str| consumes.iter().any(|c| c == ev);
    let wildcard = consumes.iter().any(|c| c == "*");

    if let Some(set) = source_event_type {
        // A precise event type the transform takes (or takes anything) — use
        // it. Otherwise fall through to the canonical candidates.
        if wildcard || accepts(set) {
            return set.to_string();
        }
    }

    if let Some(found) = sf_inputs_for(canonical_id)
        .into_iter()
        .find(|ev| accepts(ev))
    {
        return found.to_string();
    }

    source_event_type
        .map(|s| s.to_string())
        .unwrap_or_else(|| default_sf_event_for(canonical_id))
}

/// SpiderFoot emits these as side-channel diagnostics; they aren't really
/// entities and would clutter the canvas. Callers should skip persistence.
pub fn is_noise(sf_event: &str) -> bool {
    matches!(sf_event, "ERROR_MESSAGE")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whois_round_trip() {
        assert_eq!(canonical_for("DOMAIN_WHOIS"), "WhoisRecord");
    }

    #[test]
    fn unknown_falls_back_to_generic() {
        assert_eq!(canonical_for("SOME_FUTURE_TYPE"), "Generic");
    }

    #[test]
    fn every_canonical_id_in_mappings_exists_in_all() {
        let known: std::collections::HashSet<_> = ALL.iter().map(|c| c.id).collect();
        for (sf, canonical) in MAPPINGS {
            assert!(
                known.contains(canonical),
                "MAPPINGS entry {sf} → {canonical} references a canonical id not in ALL",
            );
        }
    }

    #[test]
    fn no_duplicate_sf_events_in_mappings() {
        let mut seen = std::collections::HashSet::new();
        for (sf, _) in MAPPINGS {
            assert!(seen.insert(*sf), "duplicate SF event in MAPPINGS: {sf}");
        }
    }

    #[test]
    fn sf_inputs_for_domain_includes_domain_name() {
        let inputs = sf_inputs_for("Domain");
        assert!(inputs.contains(&"DOMAIN_NAME"));
        assert!(inputs.contains(&"AFFILIATE_INTERNET_NAME"));
    }

    #[test]
    fn default_sf_event_for_domain_is_domain_name() {
        assert_eq!(default_sf_event_for("Domain"), "DOMAIN_NAME");
    }

    fn consumes(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn input_event_prefers_what_the_transform_consumes() {
        // sfp_dnsresolve takes INTERNET_NAME but not DOMAIN_NAME — a Domain
        // entity must be sent INTERNET_NAME, not the DOMAIN_NAME default.
        let ev = input_event_for_transform("Domain", None, &consumes(&["INTERNET_NAME"]));
        assert_eq!(ev, "INTERNET_NAME");
    }

    #[test]
    fn input_event_uses_domain_name_when_transform_wants_it() {
        // sfp_whois consumes DOMAIN_NAME.
        let ev = input_event_for_transform("Domain", None, &consumes(&["DOMAIN_NAME"]));
        assert_eq!(ev, "DOMAIN_NAME");
    }

    #[test]
    fn input_event_honours_source_event_type_when_consumed() {
        let ev = input_event_for_transform(
            "Domain",
            Some("AFFILIATE_INTERNET_NAME"),
            &consumes(&["AFFILIATE_INTERNET_NAME", "INTERNET_NAME"]),
        );
        assert_eq!(ev, "AFFILIATE_INTERNET_NAME");
    }

    #[test]
    fn input_event_wildcard_keeps_specific_source_type() {
        let ev = input_event_for_transform("Domain", Some("INTERNET_NAME"), &consumes(&["*"]));
        assert_eq!(ev, "INTERNET_NAME");
    }

    #[test]
    fn input_event_falls_back_to_default_when_nothing_matches() {
        // Transform consumes something unrelated → fall back to the canonical
        // default so the sidecar surfaces a clean "no events" rather than us
        // guessing wrong.
        let ev = input_event_for_transform("Domain", None, &consumes(&["IP_ADDRESS"]));
        assert_eq!(ev, "DOMAIN_NAME");
    }
}
