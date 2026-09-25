//! Operational-security classification of transforms.
//!
//! Each transform is either:
//!   - **passive**: queries third-party indices/APIs that don't touch the
//!     investigation target (Google search, HIBP, VirusTotal, Shodan
//!     [Shodan already scanned], a local face detector). Safe in
//!     surveillance / red-team contexts where target awareness matters.
//!   - **active**: directly contacts the target — DNS resolution, port
//!     scans, web spidering, content fetching, reverse-image submission
//!     to a third-party service that crawls the image again. Visible on
//!     the target's logs.
//!   - **unknown**: not classified yet. The UI surfaces this as a
//!     yellow "?" badge so the investigator can fall back on their own
//!     judgement.
//!
//! The lists below are curated by hand for the most commonly-used
//! transforms. SpiderFoot ships ~237 modules; we'll never label them
//! all — but the obvious-active ones (anything that hits the target
//! directly) should be here. PRs welcome.

use serde::Serialize;

/// Wire-format passivity classification. `&'static str` so it serialises
/// to a stable JSON string with no allocation.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Passivity {
    Passive,
    Active,
    Unknown,
}

/// Transforms that touch the target directly. Running these from a
/// stealth-required investigation should warn the user.
const ACTIVE: &[&str] = &[
    // ─── DNS / network probes (resolve target's nameservers) ──
    "sfp_dnsresolve",
    "sfp_dnsbrute",
    "sfp_dnsneighbor",
    "sfp_dnscommonsrv",
    "sfp_dnszonexfer",
    "sfp_dnsraw",
    "sfp_dnstwist",
    // ─── Port / service probes ───────────────────────────────
    "sfp_portscan_tcp",
    "sfp_portscan_basic",
    // ─── Web spidering / content fetching ────────────────────
    "sfp_spider",
    "sfp_pageinfo",
    "sfp_webframework",
    "sfp_robotstxt",
    "sfp_sitemap",
    "sfp_subdomain_takeover",
    "sfp_zonefile",
    // ─── TLS / cert direct fetch ────────────────────────────
    "sfp_sslcert",
    // ─── WHOIS / IP intel that pings the target's RIR data ──
    // (WHOIS is gray — query goes to a registrar, not the target,
    //  but the target may be alerted by some monitors. Listing as
    //  active to be safe.)
    "sfp_whois",
    "sfp_arin",
    "sfp_ripe",
    "sfp_bgpview",
    // ─── Subdomain enumeration that probes the target ───────
    "sfp_subdomain_findomain",
    "sfp_subdomain_amass",
    // ─── SMTP probes ────────────────────────────────────────
    "sfp_smtp",
    "sfp_smtp_check",
    // ─── Username / account search (queries many sites, can be
    //  noisy enough to be active) ────────────────────────────
    "sfp_accounts",
    "sfp_socialprofiles",
    // ─── Reverse-image search (submits the image to a third
    //  party that then re-crawls it). Active because the search
    //  engine sees the image. ─────────────────────────────────
    "yandex_reverse_image",
    "facecheckid_reverse_image",
    "pimeyes_reverse_image",
    "face_search_aggregate",
    // ─── net-intel reverse DNS hits the resolver / authoritative
    //  servers for the target's PTR zone. ─────────────────────
    "ip_reverse_dns",
];

/// Transforms that query third-party indices without contacting the
/// target. Safe to run in stealth investigations.
const PASSIVE: &[&str] = &[
    // ─── Local-only ─────────────────────────────────────────
    "face_detect",            // local InsightFace, no network
    "brazuca_resources",      // suggests URLs, no fetch
    // ─── Breach / reputation indices ────────────────────────
    "sfp_hibp",
    "sfp_haveibeenpwned",
    "sfp_emailrep",
    "sfp_virustotal",
    "sfp_threatcrowd",
    "sfp_threatfox",
    "sfp_abuseipdb",
    "sfp_greynoise",
    // ─── Search engines (target sees nothing) ───────────────
    "sfp_googlesearch",
    "sfp_bingsearch",
    "sfp_duckduckgo",
    // ─── Third-party scan indices (someone else scanned) ────
    "sfp_shodan",
    "sfp_censys",
    "sfp_intelx",
    "sfp_securitytrails",
    "sfp_circl_cve",
    // ─── Geo / IP enrichment via API ────────────────────────
    "sfp_ipinfo",
    "sfp_ipapi",
    "sfp_freegeoip",
    // ─── Crypto / paste sites (passive lookup) ──────────────
    "sfp_pastebin",
    "sfp_etherscan",
    "sfp_bitcoinwhoswho",
    // ─── Curated lists ──────────────────────────────────────
    "sfp_alphavantage",
    "sfp_hunter",
    "sfp_keybase",
    "sfp_gravatar",
    "sfp_github",
    // ─── First-party stdlib sidecars (query third-party indices,
    //  never the target). ────────────────────────────────────
    "crtsh_subdomains",       // crt.sh CT logs
    "crtsh_certs",            // crt.sh CT logs
    "ip_geolocate",           // ip-api.com (someone else's geo DB)
    "wayback_urls",           // Internet Archive index
    "wayback_subdomains",     // Internet Archive index
    "wikidata_lookup",        // Wikidata knowledge base
    "email_enrich",           // Gravatar + curated links, no target contact
    // ─── Brazilian sources (offline validators + third-party
    //  open-data APIs; never contact the target). ─────────────
    "brazuca_cpf_validate",   // offline checksum
    "brazuca_cnpj_validate",  // offline checksum
    "brazuca_registrobr",     // BrasilAPI registro.br index
    "brazuca_camara_deputado",// Câmara open data
    "brazuca_querido_diario", // gazette search index
];

/// Classify a transform by id. Unknown defaults to `Unknown` so the UI
/// can flag it as "judgement call required" rather than mislabelling.
pub fn classify(transform_id: &str) -> Passivity {
    if ACTIVE.iter().any(|&s| s == transform_id) {
        return Passivity::Active;
    }
    if PASSIVE.iter().any(|&s| s == transform_id) {
        return Passivity::Passive;
    }
    Passivity::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_active() {
        assert_eq!(classify("sfp_dnsresolve"), Passivity::Active);
        assert_eq!(classify("yandex_reverse_image"), Passivity::Active);
    }

    #[test]
    fn known_passive() {
        assert_eq!(classify("face_detect"), Passivity::Passive);
        assert_eq!(classify("sfp_googlesearch"), Passivity::Passive);
    }

    #[test]
    fn unknown_falls_through() {
        assert_eq!(classify("sfp_made_up_module"), Passivity::Unknown);
    }
}
