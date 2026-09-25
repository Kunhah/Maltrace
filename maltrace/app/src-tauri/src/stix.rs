//! STIX 2.1 bundle export.
//!
//! Maps maltrace canonical entities + edges into a STIX 2.1 JSON
//! bundle that TIPs (MISP, OpenCTI, ThreatConnect, …) can ingest. The
//! mapping is intentionally lossy in the conservative direction: when a
//! canonical type doesn't have an obvious STIX equivalent, we emit a
//! `note` SDO with the maltrace metadata rather than guess wrong.
//!
//! Each maltrace entity becomes one SDO/SCO; each edge becomes one
//! `relationship` SRO. Bundle ids and SDO ids are stable per case so
//! re-export over the same case produces diff-friendly output.

use crate::db::{Edge, Entity};
use serde_json::{json, Value};

pub fn build_bundle(case_name: &str, entities: &[Entity], edges: &[Edge]) -> Value {
    let mut objects: Vec<Value> = Vec::with_capacity(entities.len() + edges.len() + 1);

    // Top-level identity for the producer — required by some
    // consumers; cheap to include unconditionally.
    let producer_id = stable_uuid("identity", &format!("maltrace:{case_name}"));
    objects.push(json!({
        "type": "identity",
        "spec_version": "2.1",
        "id": producer_id,
        "name": "maltrace",
        "identity_class": "system",
        "description": format!("maltrace investigation export — case \"{case_name}\""),
    }));

    // Map maltrace entity ids → STIX ids so the relationship SROs can
    // reference them. Some maltrace types may map to multiple SCOs
    // (e.g. CryptoAddress → cryptocurrency-wallet) — but for the
    // walking-skeleton mapping we emit one STIX object per entity.
    let mut id_map: std::collections::HashMap<&str, String> =
        std::collections::HashMap::with_capacity(entities.len());

    for e in entities {
        let (object, stix_id) = map_entity(e, &producer_id);
        id_map.insert(e.id.as_str(), stix_id);
        if let Some(obj) = object {
            objects.push(obj);
        }
    }

    for edge in edges {
        let (src, tgt) = match (id_map.get(edge.source_id.as_str()), id_map.get(edge.target_id.as_str())) {
            (Some(s), Some(t)) => (s.clone(), t.clone()),
            _ => continue,
        };
        let rel_id = stable_uuid("relationship", &edge.id);
        objects.push(json!({
            "type": "relationship",
            "spec_version": "2.1",
            "id": rel_id,
            "created": edge.created_at.to_rfc3339(),
            "modified": edge.created_at.to_rfc3339(),
            "relationship_type": "related-to",
            "source_ref": src,
            "target_ref": tgt,
            "description": edge.label.clone().unwrap_or_default(),
            "created_by_ref": producer_id,
        }));
    }

    let bundle_id = stable_uuid("bundle", case_name);
    json!({
        "type": "bundle",
        "id": bundle_id,
        "objects": objects,
    })
}

/// Produce one STIX object for a maltrace entity. Returns `(object,
/// stix_id)`. Falls back to a generic `note` for canonical types we
/// don't have a clean mapping for so the relationship SROs always have
/// something to point at.
fn map_entity(e: &Entity, producer_id: &str) -> (Option<Value>, String) {
    let stix_id_for = |stix_type: &str| stable_uuid(stix_type, &e.id);
    let common = |stix_type: &str, mut extra: Value| -> Value {
        let id = stix_id_for(stix_type);
        let map = extra.as_object_mut().expect("entity map");
        map.insert("type".into(), Value::String(stix_type.to_string()));
        map.insert("spec_version".into(), Value::String("2.1".into()));
        map.insert("id".into(), Value::String(id));
        map.insert("created".into(), Value::String(e.created_at.to_rfc3339()));
        map.insert("modified".into(), Value::String(e.created_at.to_rfc3339()));
        map.insert("created_by_ref".into(), Value::String(producer_id.to_string()));
        extra
    };

    match e.entity_type.as_str() {
        "Domain" => {
            let v = common("domain-name", json!({ "value": e.label }));
            (Some(v), stix_id_for("domain-name"))
        }
        "IpAddress" => {
            let stix_type = if e.label.contains(':') {
                "ipv6-addr"
            } else {
                "ipv4-addr"
            };
            let v = common(stix_type, json!({ "value": e.label }));
            (Some(v), stix_id_for(stix_type))
        }
        "EmailAddress" => {
            let v = common("email-addr", json!({ "value": e.label }));
            (Some(v), stix_id_for("email-addr"))
        }
        "Url" => {
            let v = common("url", json!({ "value": e.label }));
            (Some(v), stix_id_for("url"))
        }
        "File" | "Image" => {
            let v = common("file", json!({ "name": e.label }));
            let id = stix_id_for("file");
            (Some(v), id)
        }
        "Person" => {
            let v = common(
                "identity",
                json!({ "name": e.label, "identity_class": "individual" }),
            );
            (Some(v), stix_id_for("identity"))
        }
        "Organization" => {
            let v = common(
                "identity",
                json!({ "name": e.label, "identity_class": "organization" }),
            );
            (Some(v), stix_id_for("identity"))
        }
        "Vulnerability" => {
            let v = common("vulnerability", json!({ "name": e.label }));
            (Some(v), stix_id_for("vulnerability"))
        }
        "ThreatIndicator" => {
            // Maltrace ThreatIndicators carry a free-form label (e.g.
            // "1.2.3.4"); STIX `indicator` wants a `pattern` — we
            // synthesise a minimal one from the label.
            let pattern = format!("[x-maltrace-indicator:value = '{}']", escape_stix(&e.label));
            let v = common(
                "indicator",
                json!({
                    "name": e.label,
                    "pattern": pattern,
                    "pattern_type": "stix",
                    "valid_from": e.created_at.to_rfc3339(),
                    "indicator_types": ["malicious-activity"],
                }),
            );
            (Some(v), stix_id_for("indicator"))
        }
        "Geolocation" => {
            let v = common("location", json!({ "name": e.label }));
            (Some(v), stix_id_for("location"))
        }
        "CryptoAddress" => {
            // STIX 2.1 has `cryptocurrency-wallet` (extension). Many
            // tools also accept it as a custom SCO. Emit minimally.
            let v = common("x-cryptocurrency-wallet", json!({ "address": e.label }));
            (Some(v), stix_id_for("x-cryptocurrency-wallet"))
        }
        "Username" => {
            let v = common("user-account", json!({ "user_id": e.label }));
            (Some(v), stix_id_for("user-account"))
        }
        "PhoneNumber" => {
            let v = common(
                "x-phone-number",
                json!({ "value": e.label }),
            );
            (Some(v), stix_id_for("x-phone-number"))
        }
        "Collection" => {
            // Collections are a maltrace UX concept — don't pollute the
            // STIX output with them; their children were re-emitted via
            // expand at insert time (or stayed inside the data column).
            // Emit a no-op id so the edge mapping doesn't break.
            (None, stix_id_for("note"))
        }
        _ => {
            // Fallback: surface as a `note` so the row isn't lost.
            let v = common(
                "note",
                json!({
                    "abstract": format!("{}: {}", e.entity_type, e.label),
                    "content": e.data.clone().unwrap_or_else(|| e.label.clone()),
                    "object_refs": [producer_id],
                }),
            );
            (Some(v), stix_id_for("note"))
        }
    }
}

/// STIX object ids must be UUIDv5-stable for diff-friendliness. We use
/// a deterministic hash of the maltrace id + STIX type so re-export of
/// the same case lands on the same ids. Returns the canonical
/// `type--uuid` form.
fn stable_uuid(stix_type: &str, key: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    stix_type.hash(&mut h);
    key.hash(&mut h);
    let v1 = h.finish();
    let mut h2 = DefaultHasher::new();
    v1.hash(&mut h2);
    stix_type.hash(&mut h2);
    let v2 = h2.finish();
    let bytes = [
        (v1 >> 56) as u8,
        (v1 >> 48) as u8,
        (v1 >> 40) as u8,
        (v1 >> 32) as u8,
        (v1 >> 24) as u8,
        (v1 >> 16) as u8,
        // RFC-4122 variant byte stuffed into byte 6 / 8 — pseudo-v4.
        ((v1 >> 8) as u8 & 0x0F) | 0x40,
        v1 as u8,
        ((v2 >> 56) as u8 & 0x3F) | 0x80,
        (v2 >> 48) as u8,
        (v2 >> 40) as u8,
        (v2 >> 32) as u8,
        (v2 >> 24) as u8,
        (v2 >> 16) as u8,
        (v2 >> 8) as u8,
        v2 as u8,
    ];
    format!(
        "{}--{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        stix_type,
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5],
        bytes[6], bytes[7],
        bytes[8], bytes[9],
        bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    )
}

fn escape_stix(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}
