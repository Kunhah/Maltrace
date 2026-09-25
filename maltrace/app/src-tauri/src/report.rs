//! Templated HTML case report.
//!
//! Produces a single self-contained HTML document with:
//!   1. Summary header (case name, counts, generated-at)
//!   2. Key findings (bookmarked entities, each with notes)
//!   3. Entities by type (counts + sample labels)
//!   4. Provenance chain (transform runs, each with new-entity summary)
//!   5. Full entity list (for completeness)
//!
//! HTML so it opens in any browser and prints cleanly to PDF via the
//! browser's own engine. PDF generation lives in the frontend (jspdf
//! already there) and stays a thin wrapper around this report's
//! sections.

use crate::db::{Case, Entity, TransformRun};

pub fn build_html(case: &Case, entities: &[Entity], runs: &[TransformRun]) -> String {
    let mut s = String::with_capacity(4096 + entities.len() * 200);
    s.push_str("<!DOCTYPE html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\n");
    s.push_str(&format!(
        "<title>{} — maltrace report</title>\n",
        html_escape(&case.name)
    ));
    s.push_str("<style>\n");
    s.push_str(REPORT_CSS);
    s.push_str("</style>\n</head><body>\n");

    // ─── Header ─────────────────────────────────────────────
    s.push_str(&format!(
        "<header class=\"r-header\"><h1>{}</h1><div class=\"r-meta\">",
        html_escape(&case.name)
    ));
    s.push_str(&format!(
        "<span>Case opened {}</span>",
        case.created_at.format("%Y-%m-%d %H:%M")
    ));
    let generated_at = chrono::Utc::now();
    s.push_str(&format!(
        "<span>Generated {}</span>",
        generated_at.format("%Y-%m-%d %H:%M UTC")
    ));
    s.push_str(&format!(
        "<span><strong>{}</strong> entities · <strong>{}</strong> transform runs</span>",
        entities.len(),
        runs.len()
    ));
    if case.locked {
        s.push_str("<span class=\"r-locked\">🔒 Evidence locked</span>");
    }
    s.push_str("</div></header>\n");

    // ─── Key findings (bookmarked first, with notes) ────────
    let bookmarked: Vec<&Entity> = entities.iter().filter(|e| e.bookmarked).collect();
    if !bookmarked.is_empty() {
        s.push_str("<section class=\"r-section\"><h2>Key findings</h2><ul class=\"r-findings\">\n");
        for e in &bookmarked {
            s.push_str(&format!(
                "<li><div class=\"r-finding-head\"><span class=\"r-type\">{}</span> <strong>{}</strong></div>",
                html_escape(&e.entity_type),
                html_escape(&e.label),
            ));
            if let Some(n) = e.notes.as_deref().filter(|s| !s.trim().is_empty()) {
                s.push_str(&format!(
                    "<div class=\"r-notes\">{}</div>",
                    html_escape(n).replace('\n', "<br/>")
                ));
            }
            if let Some(url) = e.source_url.as_deref() {
                s.push_str(&format!(
                    "<div class=\"r-source\">Source: <a href=\"{u}\">{u}</a></div>",
                    u = html_escape(url)
                ));
            }
            s.push_str("</li>\n");
        }
        s.push_str("</ul></section>\n");
    }

    // ─── Entities by type ───────────────────────────────────
    let mut by_type: std::collections::HashMap<&str, Vec<&Entity>> =
        std::collections::HashMap::new();
    for e in entities {
        by_type.entry(e.entity_type.as_str()).or_default().push(e);
    }
    let mut type_order: Vec<(&str, Vec<&Entity>)> = by_type.into_iter().collect();
    type_order.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(b.0)));

    s.push_str("<section class=\"r-section\"><h2>Entities by type</h2>\n");
    s.push_str("<table class=\"r-table\"><thead><tr><th>Type</th><th>Count</th><th>Examples</th></tr></thead><tbody>\n");
    for (ty, items) in &type_order {
        let examples = items
            .iter()
            .take(5)
            .map(|e| html_escape(&e.label))
            .collect::<Vec<_>>()
            .join(", ");
        let more = if items.len() > 5 {
            format!(" + {} more", items.len() - 5)
        } else {
            String::new()
        };
        s.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}{}</td></tr>\n",
            html_escape(ty),
            items.len(),
            examples,
            more
        ));
    }
    s.push_str("</tbody></table></section>\n");

    // ─── Provenance chain ───────────────────────────────────
    s.push_str("<section class=\"r-section\"><h2>Provenance chain</h2>\n");
    if runs.is_empty() {
        s.push_str("<p class=\"r-muted\">No transform runs recorded — every entity is manually added.</p>\n");
    } else {
        s.push_str(
            "<table class=\"r-table\"><thead><tr><th>#</th><th>When</th><th>Transform</th><th>Seed</th><th>Result</th></tr></thead><tbody>\n",
        );
        for r in runs.iter().rev() {
            let status_class = if r.status == "ok" { "r-ok" } else { "r-err" };
            let result = if r.status == "ok" {
                format!("+{} entities", r.new_entity_count)
            } else {
                format!(
                    "ERROR — {}",
                    html_escape(r.error_message.as_deref().unwrap_or("unknown"))
                )
            };
            s.push_str(&format!(
                "<tr class=\"{cls}\"><td>{id}</td><td>{when}</td><td><code>{t}</code></td><td>{seed}</td><td>{res}</td></tr>\n",
                cls = status_class,
                id = r.id,
                when = r.started_at.format("%Y-%m-%d %H:%M"),
                t = html_escape(&r.transform_id),
                seed = html_escape(r.source_label.as_deref().unwrap_or("—")),
                res = result,
            ));
        }
        s.push_str("</tbody></table></section>\n");
    }

    // ─── Full entity table ─────────────────────────────────
    s.push_str("<section class=\"r-section\"><h2>All entities</h2>\n");
    s.push_str("<table class=\"r-table r-compact\"><thead><tr><th>Type</th><th>Label</th><th>Created</th><th>Source</th></tr></thead><tbody>\n");
    for e in entities {
        s.push_str(&format!(
            "<tr><td>{ty}</td><td>{label}</td><td>{when}</td><td>{src}</td></tr>\n",
            ty = html_escape(&e.entity_type),
            label = html_escape(&e.label),
            when = e.created_at.format("%Y-%m-%d %H:%M"),
            src = e.created_by_transform_id.as_deref().map(html_escape).unwrap_or_else(|| "manual".to_string()),
        ));
    }
    s.push_str("</tbody></table></section>\n");

    s.push_str("<footer class=\"r-footer\">Generated by maltrace.</footer>\n");
    s.push_str("</body></html>\n");
    s
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

const REPORT_CSS: &str = r#"
:root { color-scheme: light; }
body { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif; color: #0f172a; background: #f8fafc; margin: 0; padding: 0; }
.r-header { background: #0f172a; color: #f1f5f9; padding: 28px 40px; }
.r-header h1 { margin: 0 0 8px; font-size: 28px; font-weight: 600; letter-spacing: -0.01em; }
.r-meta { display: flex; flex-wrap: wrap; gap: 18px; font-size: 13px; color: #cbd5e1; }
.r-meta strong { color: #f1f5f9; }
.r-locked { background: #fbbf24; color: #422006; padding: 2px 8px; border-radius: 4px; font-weight: 600; }
.r-section { padding: 24px 40px; border-bottom: 1px solid #e2e8f0; max-width: 1200px; margin: 0 auto; }
.r-section h2 { font-size: 18px; font-weight: 600; margin: 0 0 14px; color: #0f172a; }
.r-findings { list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 14px; }
.r-findings li { background: white; border: 1px solid #e2e8f0; border-left: 4px solid #fbbf24; padding: 12px 16px; border-radius: 6px; }
.r-finding-head { font-size: 14px; }
.r-type { display: inline-block; background: #e2e8f0; color: #334155; padding: 1px 7px; border-radius: 3px; font-size: 11px; text-transform: uppercase; margin-right: 8px; letter-spacing: 0.04em; }
.r-notes { margin-top: 6px; font-size: 13px; color: #475569; }
.r-source { margin-top: 6px; font-size: 12px; }
.r-source a { color: #2563eb; }
.r-table { width: 100%; border-collapse: collapse; font-size: 13px; background: white; border-radius: 6px; overflow: hidden; box-shadow: 0 0 0 1px #e2e8f0; }
.r-table th, .r-table td { padding: 8px 12px; text-align: left; }
.r-table thead { background: #f1f5f9; }
.r-table tbody tr:nth-child(even) { background: #f8fafc; }
.r-table.r-compact td, .r-table.r-compact th { padding: 6px 10px; font-size: 12px; }
.r-table .r-ok td:first-child { border-left: 3px solid #10b981; }
.r-table .r-err td:first-child { border-left: 3px solid #dc2626; }
.r-table code { background: #e2e8f0; padding: 1px 6px; border-radius: 3px; font-size: 12px; }
.r-muted { color: #64748b; font-style: italic; }
.r-footer { text-align: center; padding: 24px; color: #94a3b8; font-size: 12px; }
@media print {
  body { background: white; }
  .r-section { page-break-inside: avoid; }
  .r-header { background: white; color: #0f172a; border-bottom: 2px solid #0f172a; }
  .r-header h1, .r-meta strong { color: #0f172a; }
  .r-meta { color: #475569; }
}
"#;
