//! GeoJSON + KML export of geolocated entities.
//!
//! Mirrors the coordinate-extraction logic in the frontend `GeoView`:
//!   - `entity.label` matching `"LAT,LNG"` (PHYSICAL_COORDINATES events)
//!   - structured `data.{lat,lng | latitude,longitude}` /
//!     `data.extra.{lat,lng | latitude,longitude}`
//!   - fallback `data.full_data` regex match on `"LAT,LNG"`
//!
//! GeoJSON output is one `FeatureCollection` with one Point feature
//! per entity. KML output is the equivalent `<Document>` + `<Placemark>`
//! pyramid. Both omit entities without coordinates.

use crate::db::Entity;

/// Extracted geolocation for one entity. Mirrors the frontend
/// GeoView's parser so backend exports and the in-app map agree.
struct Point<'a> {
    entity: &'a Entity,
    lat: f64,
    lng: f64,
    /// Best-effort label for the popup / placemark name.
    popup: String,
}

pub fn export_geojson(entities: &[Entity]) -> String {
    let points = extract_points(entities);
    let features: Vec<serde_json::Value> = points
        .iter()
        .map(|p| {
            serde_json::json!({
                "type": "Feature",
                "id": p.entity.id,
                "geometry": {
                    "type": "Point",
                    // GeoJSON is [lng, lat], not [lat, lng].
                    "coordinates": [p.lng, p.lat],
                },
                "properties": {
                    "name": p.popup,
                    "label": p.entity.label,
                    "entity_type": p.entity.entity_type,
                    "bookmarked": p.entity.bookmarked,
                    "source_url": p.entity.source_url,
                },
            })
        })
        .collect();
    let bundle = serde_json::json!({
        "type": "FeatureCollection",
        "features": features,
    });
    serde_json::to_string_pretty(&bundle).unwrap_or_else(|_| "{}".to_string())
}

pub fn export_kml(case_name: &str, entities: &[Entity]) -> String {
    let points = extract_points(entities);
    let mut out = String::with_capacity(2048 + points.len() * 256);
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<kml xmlns=\"http://www.opengis.net/kml/2.2\">\n");
    out.push_str("  <Document>\n");
    out.push_str(&format!(
        "    <name>{}</name>\n",
        xml_escape(case_name)
    ));
    out.push_str(&format!(
        "    <description>maltrace geo export — {} placemark(s)</description>\n",
        points.len()
    ));
    for p in &points {
        out.push_str("    <Placemark>\n");
        out.push_str(&format!("      <name>{}</name>\n", xml_escape(&p.popup)));
        out.push_str(&format!(
            "      <description><![CDATA[<b>{}</b><br/>{}]]></description>\n",
            xml_escape(&p.entity.entity_type),
            xml_escape(&p.entity.label),
        ));
        out.push_str("      <Point>\n");
        out.push_str(&format!(
            "        <coordinates>{},{},0</coordinates>\n",
            p.lng, p.lat
        ));
        out.push_str("      </Point>\n");
        out.push_str("    </Placemark>\n");
    }
    out.push_str("  </Document>\n");
    out.push_str("</kml>\n");
    out
}

fn extract_points(entities: &[Entity]) -> Vec<Point<'_>> {
    entities
        .iter()
        .filter_map(|e| point_for(e))
        .collect()
}

fn point_for(e: &Entity) -> Option<Point<'_>> {
    // (1) "LAT,LNG" in the label.
    if let Some((lat, lng)) = parse_lat_lng(&e.label) {
        return Some(Point {
            entity: e,
            lat,
            lng,
            popup: e.label.clone(),
        });
    }
    // (2)+(3) Structured data + full_data fallback.
    if let Some(data) = e.data.as_deref() {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data) {
            if let Some(extra) = parsed.get("extra").and_then(|v| v.as_object()) {
                if let Some(pair) = numeric_pair(extra.get("lat"), extra.get("lng"))
                    .or_else(|| numeric_pair(extra.get("latitude"), extra.get("longitude")))
                {
                    let popup = extra
                        .get("title")
                        .and_then(|v| v.as_str())
                        .or_else(|| extra.get("address").and_then(|v| v.as_str()))
                        .map(String::from)
                        .unwrap_or_else(|| e.label.clone());
                    return Some(Point {
                        entity: e,
                        lat: pair.0,
                        lng: pair.1,
                        popup,
                    });
                }
            }
            if let Some(pair) = numeric_pair(parsed.get("lat"), parsed.get("lng"))
                .or_else(|| numeric_pair(parsed.get("latitude"), parsed.get("longitude")))
            {
                let popup = parsed
                    .get("full_data")
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .unwrap_or_else(|| e.label.clone());
                return Some(Point {
                    entity: e,
                    lat: pair.0,
                    lng: pair.1,
                    popup,
                });
            }
            if let Some(full) = parsed.get("full_data").and_then(|v| v.as_str()) {
                if let Some((lat, lng)) = parse_lat_lng(full) {
                    return Some(Point {
                        entity: e,
                        lat,
                        lng,
                        popup: full.to_string(),
                    });
                }
            }
        }
    }
    None
}

fn numeric_pair(a: Option<&serde_json::Value>, b: Option<&serde_json::Value>) -> Option<(f64, f64)> {
    let av = a?.as_f64().or_else(|| a?.as_str().and_then(|s| s.parse().ok()))?;
    let bv = b?.as_f64().or_else(|| b?.as_str().and_then(|s| s.parse().ok()))?;
    if av.abs() <= 90.0 && bv.abs() <= 180.0 {
        Some((av, bv))
    } else {
        None
    }
}

/// Same regex shape as the frontend: "LAT,LNG" with optional "()" and
/// scientific notation. Rejects out-of-range pairs.
fn parse_lat_lng(s: &str) -> Option<(f64, f64)> {
    let trimmed = s.trim().trim_start_matches('(').trim_end_matches(')');
    let mut parts = trimmed.split(',');
    let a = parts.next()?.trim();
    let b = parts.next()?.trim();
    if parts.next().is_some() {
        return None;
    }
    let lat: f64 = a.parse().ok()?;
    let lng: f64 = b.parse().ok()?;
    if !lat.is_finite() || !lng.is_finite() || lat.abs() > 90.0 || lng.abs() > 180.0 {
        return None;
    }
    Some((lat, lng))
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
