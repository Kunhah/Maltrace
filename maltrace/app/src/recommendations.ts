/// Investigation-flow helpers: best-effort guesses about what an entity is
/// from a raw user value, and the small hand-curated set of transforms that
/// are usually the first thing you'd run on a given canonical type.
///
/// These power three "discoverability" affordances in the UI:
///   - Type auto-detection in the topbar `AddEntityForm`
///   - "Try with…" example chips on the empty canvas
///   - "Recommended" group at the top of the entity inspector's transform list
///
/// All matchers are intentionally simple — false positives are cheap (the
/// user can override the dropdown), false negatives just mean they pick
/// the type by hand. None of the regexes call out to the backend.

import type { CanonicalType } from "./types";

/// Single-pass guess of the canonical type id (e.g. "Domain") for a raw
/// string. Returns null when nothing matches confidently. Order matters:
/// more specific patterns first.
export function detectCanonicalType(raw: string): string | null {
  const v = raw.trim();
  if (!v) return null;

  // URL: scheme + ://
  if (/^https?:\/\//i.test(v)) return "Url";

  // Email: simple "x@y.z" check — good enough to outvote Domain.
  if (/^[^\s@]+@[^\s@]+\.[^\s@]{2,}$/.test(v)) return "EmailAddress";

  // CIDR / netblock: x.x.x.x/y or IPv6 with /.
  if (/^[0-9a-f:.]+\/\d{1,3}$/i.test(v) && v.includes("/")) return "Netblock";

  // IPv4
  if (/^(?:\d{1,3}\.){3}\d{1,3}$/.test(v)) {
    const parts = v.split(".").map(Number);
    if (parts.every((n) => n >= 0 && n <= 255)) return "IpAddress";
  }
  // IPv6 (loose)
  if (/^[0-9a-f:]+$/i.test(v) && v.includes(":") && v.length > 2) {
    return "IpAddress";
  }

  // ASN: "AS12345" or "as12345"
  if (/^as\d{1,10}$/i.test(v)) return "Asn";

  // Crypto: 0x-prefixed 40-hex (ETH) or BTC-shaped strings.
  if (/^0x[0-9a-fA-F]{40}$/.test(v)) return "CryptoAddress";
  if (/^(bc1|[13])[a-zA-HJ-NP-Z0-9]{25,62}$/.test(v)) return "CryptoAddress";

  // Hashes: MD5 / SHA1 / SHA256 — hex of fixed length.
  if (/^[0-9a-f]{32}$/i.test(v)) return "Hash";
  if (/^[0-9a-f]{40}$/i.test(v)) return "Hash";
  if (/^[0-9a-f]{64}$/i.test(v)) return "Hash";

  // Phone: starts with +, or is mostly digits with 8-15 of them.
  if (/^\+\d[\d\s().-]{6,}$/.test(v)) return "PhoneNumber";

  // Username: @handle
  if (/^@[A-Za-z0-9_.-]{2,}$/.test(v)) return "Username";

  // Domain: contains a dot and a plausible TLD; reject obvious sentences.
  if (
    /^[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?(?:\.[A-Za-z0-9-]+)+$/.test(v) &&
    !v.includes(" ")
  ) {
    return "Domain";
  }

  // Multi-word capitalised: probably a person's name.
  if (/^[A-Z][a-z]+(?:\s+[A-Z][a-z]+)+$/.test(v)) return "Person";

  return null;
}

/// Hand-curated "best first move" transforms for each canonical type. The
/// inspector intersects this list with the live catalogue, so unknown IDs
/// are silently skipped (the lists are forward-compatible with new
/// plugins). Order matters — top of the list shows first.
const RECOMMENDED: Record<string, readonly string[]> = {
  Domain: [
    "sfp_whois",
    "sfp_dnsresolve",
    "sfp_dnsbrute",
    "sfp_sslcert",
    "sfp_subdomain_takeover",
  ],
  IpAddress: ["sfp_ripe", "sfp_ipinfo", "sfp_bgpview", "sfp_abusech"],
  Netblock: ["sfp_ripe", "sfp_bgpview"],
  Asn: ["sfp_bgpview", "sfp_ripe"],
  EmailAddress: [
    "sfp_haveibeenpwned",
    "sfp_gravatar",
    "sfp_accounts",
    "sfp_hunter",
  ],
  Username: ["sfp_accounts", "sfp_socialprofiles"],
  Person: [
    "sfp_socialprofiles",
    "sfp_accounts",
    "sfp_ahmia",
    "brazuca_resources",
  ],
  PhoneNumber: ["sfp_numverify", "brazuca_resources"],
  Url: ["sfp_pageinfo", "sfp_screenshot", "sfp_archiveorg"],
  Image: ["face_detect", "face_search_all"],
  Face: ["face_search_yandex", "face_search_lens", "face_search_facecheck"],
};

export function recommendedTransformIds(canonicalTypeId: string): readonly string[] {
  return RECOMMENDED[canonicalTypeId] ?? [];
}

/// Example targets shown on the empty canvas. Each one drops a real entity
/// the user can immediately right-click to pivot from. Kept small and
/// covers the common "what kind of investigation am I starting" buckets.
export interface ExampleTarget {
  /// Canonical type id — must exist in the ontology.
  entityType: string;
  label: string;
  /// What this target demonstrates, surfaced as the chip's tooltip.
  hint: string;
}

export const EXAMPLE_TARGETS: readonly ExampleTarget[] = [
  {
    entityType: "Domain",
    label: "github.com",
    hint: "Start with a domain — WHOIS, DNS, subdomains.",
  },
  {
    entityType: "IpAddress",
    label: "8.8.8.8",
    hint: "Start with an IP — ASN, geolocation, reverse hosts.",
  },
  {
    entityType: "EmailAddress",
    label: "security@github.com",
    hint: "Start with an email — breach lookups, gravatar, account search.",
  },
];

/// Pretty-print a canonical type id using its ontology label, falling back
/// to the raw id when the type isn't loaded yet.
export function canonicalLabel(
  id: string,
  canonicalTypes: CanonicalType[],
): string {
  return canonicalTypes.find((t) => t.id === id)?.label ?? id;
}
