"""
maltrace ↔ wikidata adapter.

Resolve a Person or Organization name to a Wikidata item and pull structured
facts plus related-entity pivots, against the free Wikidata API (no key).

Pipeline:
  1. wbsearchentities      name → candidate items (Q-ids) ranked by relevance.
  2. wbgetentities         top item → claims + sitelinks + labels.
  3. wbgetentities (batch) resolve the Q-ids referenced by relation claims to
                           human labels so we can emit named related entities.

Emitted:
  * URL              official website (P856), Wikidata page, Wikipedia page.
  * COUNTRY_NAME     country (P17) / country of citizenship (P27).
  * COMPANY_NAME     org-typed relations (employer, parent org, subsidiary…).
  * HUMAN_NAME       person-typed relations (founder, CEO, board member…).
  * RAW_RIR_DATA     a summary + the raw claim digest.

Transforms:
    wikidata_lookup     Person/Organization name → the above.

One-shot JSON protocol shared with the other sidecars.
"""

from __future__ import annotations

import argparse
import json
import sys
import traceback
import urllib.error
import urllib.parse
import urllib.request
from typing import Any, Dict, List, Optional


_API = "https://www.wikidata.org/w/api.php"
_HEADERS = {
    "User-Agent": "maltrace-wikidata/0.1 (+https://github.com/)",
    "Accept": "application/json",
}
_DEFAULT_TIMEOUT = 15

# Relation properties → (target ontology event type, human label).
# These are the claims that point at *another entity* worth pivoting on.
_PERSON_RELATIONS = {
    "P112": "founder",
    "P169": "chief executive officer",
    "P1037": "director / manager",
    "P3320": "board member",
    "P488": "chairperson",
}
_ORG_RELATIONS = {
    "P108": "employer",
    "P749": "parent organization",
    "P355": "subsidiary",
    "P127": "owned by",
    "P1830": "owner of",
    "P159": "headquarters location",
}
_COUNTRY_PROPS = ("P17", "P27")        # country, country of citizenship
_WEBSITE_PROP = "P856"                 # official website
_INSTANCE_OF = "P31"


def _api_get(params: Dict[str, str], *, timeout: float) -> Any:
    params = {**params, "format": "json"}
    url = _API + "?" + urllib.parse.urlencode(params)
    req = urllib.request.Request(url, headers=_HEADERS)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            raw = resp.read()
    except (urllib.error.HTTPError, urllib.error.URLError,
            TimeoutError, ConnectionError) as e:
        return {"_error": str(e)}
    try:
        return json.loads(raw.decode("utf-8", errors="replace"))
    except json.JSONDecodeError:
        return {"_error": "non-JSON response"}


def _search(name: str, *, timeout: float) -> List[dict]:
    body = _api_get({
        "action": "wbsearchentities",
        "search": name,
        "language": "en",
        "uselang": "en",
        "type": "item",
        "limit": "5",
    }, timeout=timeout)
    return body.get("search", []) if isinstance(body, dict) else []


def _get_entities(ids: List[str], *, props: str, timeout: float) -> Dict[str, dict]:
    if not ids:
        return {}
    out: Dict[str, dict] = {}
    # wbgetentities accepts up to 50 ids per call.
    for i in range(0, len(ids), 50):
        chunk = ids[i:i + 50]
        body = _api_get({
            "action": "wbgetentities",
            "ids": "|".join(chunk),
            "props": props,
            "languages": "en",
        }, timeout=timeout)
        if isinstance(body, dict):
            out.update(body.get("entities", {}) or {})
    return out


def _claim_targets(entity: dict, prop: str) -> List[str]:
    """Return the Q-ids referenced by a property's claims."""
    targets: List[str] = []
    for claim in (entity.get("claims", {}) or {}).get(prop, []):
        snak = (claim.get("mainsnak", {}) or {}).get("datavalue", {})
        val = snak.get("value")
        if isinstance(val, dict) and val.get("entity-type") == "item":
            qid = val.get("id")
            if qid:
                targets.append(qid)
    return targets


def _claim_strings(entity: dict, prop: str) -> List[str]:
    """Return string-valued claims (e.g. official website URLs)."""
    out: List[str] = []
    for claim in (entity.get("claims", {}) or {}).get(prop, []):
        snak = (claim.get("mainsnak", {}) or {}).get("datavalue", {})
        val = snak.get("value")
        if isinstance(val, str):
            out.append(val)
    return out


def _label_of(entity: dict) -> Optional[str]:
    return (((entity.get("labels", {}) or {}).get("en", {}) or {}).get("value"))


def _desc_of(entity: dict) -> Optional[str]:
    return (((entity.get("descriptions", {}) or {}).get("en", {}) or {}).get("value"))


def _is_org(instance_labels: List[str]) -> bool:
    blob = " ".join(instance_labels).lower()
    return any(w in blob for w in (
        "business", "company", "enterprise", "organization", "organisation",
        "agency", "corporation", "institution", "nonprofit", "university"))


# ──────────────────────────────────────────────────────────────────────
#  Main transform
# ──────────────────────────────────────────────────────────────────────

def _run_lookup(event_type: str, data: str, opts: dict) -> List[dict]:
    name = (data or "").strip()
    timeout = float(opts.get("timeout", _DEFAULT_TIMEOUT))
    if not name:
        return [_raw("wikidata_lookup: empty input", event_type,
                     "wikidata_lookup", 30, {"input": data})]

    hits = _search(name, timeout=timeout)
    if not hits:
        return [_raw(f"Wikidata: no item matches {name!r}", event_type,
                     "wikidata_lookup", 40, {"name": name})]

    top = hits[0]
    qid = top.get("id")
    ent_map = _get_entities([qid], props="claims|labels|descriptions|sitelinks",
                            timeout=timeout)
    entity = ent_map.get(qid, {})

    events: List[dict] = []

    # ── instance-of (what kind of thing this is) ─────────────────────
    instance_qids = _claim_targets(entity, _INSTANCE_OF)

    # ── Gather every related Q-id we want to resolve to a label ──────
    relation_props = {**_PERSON_RELATIONS, **_ORG_RELATIONS}
    related: Dict[str, List[str]] = {}   # prop → [qid, …]
    all_related_qids: List[str] = list(instance_qids)
    for prop in relation_props:
        tgts = _claim_targets(entity, prop)
        if tgts:
            related[prop] = tgts
            all_related_qids.extend(tgts)
    for prop in _COUNTRY_PROPS:
        tgts = _claim_targets(entity, prop)
        if tgts:
            related.setdefault(prop, []).extend(tgts)
            all_related_qids.extend(tgts)

    label_map = _get_entities(sorted(set(all_related_qids)),
                              props="labels", timeout=timeout) if all_related_qids else {}

    def lbl(q: str) -> Optional[str]:
        return _label_of(label_map.get(q, {})) or q

    instance_labels = [lbl(q) for q in instance_qids]
    subject_is_org = _is_org(instance_labels)

    # ── Wikidata + Wikipedia URLs ────────────────────────────────────
    events.append({
        "event_type": "URL",
        "data": f"https://www.wikidata.org/wiki/{qid}",
        "module": "wikidata_lookup",
        "confidence": 90,
        "source_event_type": event_type,
        "extra": {"url": f"https://www.wikidata.org/wiki/{qid}",
                  "title": f"Wikidata: {top.get('label')}",
                  "description": top.get("description"), "qid": qid},
    })
    sitelinks = entity.get("sitelinks", {}) or {}
    enwiki = sitelinks.get("enwiki")
    if enwiki and enwiki.get("title"):
        wp = "https://en.wikipedia.org/wiki/" + urllib.parse.quote(
            enwiki["title"].replace(" ", "_"))
        events.append({
            "event_type": "URL",
            "data": wp,
            "module": "wikidata_lookup",
            "confidence": 88,
            "source_event_type": event_type,
            "extra": {"url": wp, "title": f"Wikipedia: {enwiki['title']}",
                      "qid": qid},
        })

    # ── Official website ─────────────────────────────────────────────
    for site in _claim_strings(entity, _WEBSITE_PROP):
        events.append({
            "event_type": "URL",
            "data": site,
            "module": "wikidata_lookup",
            "confidence": 85,
            "source_event_type": event_type,
            "extra": {"url": site, "title": "Official website",
                      "relation": "official website (P856)", "qid": qid},
        })

    # ── Country ──────────────────────────────────────────────────────
    country_qids = []
    for prop in _COUNTRY_PROPS:
        country_qids.extend(related.get(prop, []))
    for cq in dict.fromkeys(country_qids):
        events.append({
            "event_type": "COUNTRY_NAME",
            "data": lbl(cq),
            "module": "wikidata_lookup",
            "confidence": 80,
            "source_event_type": event_type,
            "extra": {"qid": cq, "relation": "country", "source": "wikidata"},
        })

    # ── Related people / organisations (pivots) ──────────────────────
    for prop, role in relation_props.items():
        for tq in related.get(prop, []):
            target_label = lbl(tq)
            if not target_label or target_label == tq:
                continue
            # Person-relation props → HUMAN_NAME; org-relation props →
            # COMPANY_NAME. headquarters (P159) is a place → COUNTRY-ish, but
            # we keep it as an Organization-adjacent place name fallback.
            is_person_rel = prop in _PERSON_RELATIONS
            evt = "HUMAN_NAME" if is_person_rel else "COMPANY_NAME"
            events.append({
                "event_type": evt,
                "data": target_label,
                "module": "wikidata_lookup",
                "confidence": 75,
                "source_event_type": event_type,
                "extra": {"qid": tq, "relation": role,
                          "wikidata_url": f"https://www.wikidata.org/wiki/{tq}",
                          "source": "wikidata"},
            })

    # ── Summary ──────────────────────────────────────────────────────
    events.append(_raw(
        f"Wikidata {qid} ({top.get('label')}): {top.get('description') or '—'} "
        f"· {len([e for e in events if e['event_type'] in ('HUMAN_NAME','COMPANY_NAME')])} relation(s)",
        event_type, "wikidata_lookup", 75,
        {"qid": qid, "label": top.get("label"),
         "description": top.get("description"),
         "instance_of": instance_labels,
         "classified_as": "organization" if subject_is_org else "person/other",
         "candidates": [{"id": h.get("id"), "label": h.get("label"),
                         "description": h.get("description")} for h in hits]}))
    return events


def _raw(msg: str, event_type: str, module: str, conf: int, extra: dict) -> dict:
    return {
        "event_type": "RAW_RIR_DATA",
        "data": msg,
        "module": module,
        "confidence": conf,
        "source_event_type": event_type,
        "extra": extra,
    }


# ──────────────────────────────────────────────────────────────────────
#  Manifest + dispatch
# ──────────────────────────────────────────────────────────────────────

TRANSFORMS = [
    {
        "id": "wikidata_lookup",
        "name": "Wikidata enrichment (facts + relations)",
        "description": (
            "Resolve a person or organisation name to a Wikidata item and "
            "emit its official website, Wikipedia page, country, and related "
            "entities (employer, founders, CEO, parent company, subsidiaries) "
            "as pivotable nodes. No API key."
        ),
        "categories": ["People", "Organization", "Enrichment"],
        "group": [],
        "consumes": ["HUMAN_NAME", "COMPANY_NAME", "AFFILIATE_COMPANY_NAME"],
        "produces": ["URL", "COUNTRY_NAME", "COMPANY_NAME", "HUMAN_NAME",
                     "RAW_RIR_DATA"],
    },
]


def _dispatch_run(module: str, event_type: str, data: str, opts: dict) -> List[dict]:
    if module == "wikidata_lookup":
        return _run_lookup(event_type, data, opts)
    raise ValueError(f"unknown module: {module}")


def cmd_list_transforms(_: argparse.Namespace) -> int:
    json.dump({"transforms": TRANSFORMS, "skipped": []}, sys.stdout)
    sys.stdout.write("\n")
    return 0


def cmd_run(args: argparse.Namespace) -> int:
    opts: dict = {}
    if args.opts:
        try:
            opts = json.loads(args.opts)
            if not isinstance(opts, dict):
                raise ValueError("--opts must be a JSON object")
        except (json.JSONDecodeError, ValueError) as exc:
            json.dump({"ok": False, "error": f"bad --opts: {exc}", "events": []},
                      sys.stdout)
            sys.stdout.write("\n")
            return 2

    try:
        events = _dispatch_run(args.module, args.event_type, args.data, opts)
    except Exception as exc:  # noqa: BLE001
        json.dump({"ok": False, "error": f"{type(exc).__name__}: {exc}",
                   "traceback": traceback.format_exc(), "events": []}, sys.stdout)
        sys.stdout.write("\n")
        return 3

    json.dump({"ok": True, "events": events}, sys.stdout)
    sys.stdout.write("\n")
    return 0


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(prog="maltrace-wikidata-adapter")
    sub = parser.add_subparsers(dest="cmd", required=True)
    sub.add_parser("list-transforms", help="List wikidata transforms.")
    run = sub.add_parser("run", help="Look up a name in Wikidata.")
    run.add_argument("--module", required=True)
    run.add_argument("--event-type", required=True)
    run.add_argument("--data", required=True)
    run.add_argument("--opts", default=None, help='JSON: {"timeout": 15}')
    args = parser.parse_args(argv)
    if args.cmd == "list-transforms":
        return cmd_list_transforms(args)
    if args.cmd == "run":
        return cmd_run(args)
    parser.print_help(sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())
