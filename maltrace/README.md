# maltrace

> Open-source graph-based OSINT investigation app — a Maltego CE alternative.

maltrace is a desktop app for visual OSINT investigations. You drop an
entity on the canvas (a domain, email, IP, image, person…), run
**transforms** that return related entities, and pivot through the
resulting graph. The original Maltego CE is closed-source and limited;
maltrace is MIT-licensed and extensible.

---

## What's in the box

| Capability | What it does |
|---|---|
| **230+ transforms** | Bundled SpiderFoot adapter — WHOIS, DNS, IP intel, breach lookups, ASN data, dark-web mentions, social-media enumeration, threat-intel correlation. |
| **Face search** | InsightFace face detection + four reverse-image backends (Yandex, Bing Visual, Google Lens, FaceCheck.ID). Find where else a face appears on the web. |
| **Brazilian OSINT** | 60+ curated portals (Receita Federal, TSE, JusBrasil, CVM, COAF, BCB, tribunais, INPI, CADE, IBAMA, TCU, Câmara/Senado, SINTEGRA, CENSEC…) **plus live + offline transforms**: BrasilAPI (CNPJ / CEP / DDD / bancos / registro.br), ViaCEP, Hudson Rock Cavalier infostealer leaks, **offline CPF/CNPJ check-digit validation** (with CPF fiscal-region inference), **Câmara dos Deputados PEP screening**, and **Querido Diário** municipal-gazette full-text search. All key-free. |
| **Username enumeration** | Sherlock-style probe: given a `USERNAME`, hit ~40 curated social, dev, media and gaming sites (GitHub, GitLab, DockerHub, Steam, Patreon, Mastodon, Lichess…) in parallel and emit one URL hit per match. No API keys; stable detection rules (HTTP status + body markers) hold false-positive rate near zero. |
| **Crypto wallet OSINT** | Detects BTC/ETH addresses and pulls balance, tx counts, ERC-20 holdings via mempool.space and ethplorer (no key required). Also surfaces curated reputation pivots (Chainabuse, BitcoinAbuse, Etherscan, OXT). |
| **Certificate Transparency** | Mines crt.sh CT logs for a domain: subdomains that DNS brute-force misses, plus the issued certificates and issuing CAs/organisations. Passive, no key. |
| **Network intelligence** | Geolocates IPs via ip-api.com — country, city, **lat/lng pins that show up on the Map view** — plus hosting provider, ASN and reverse-DNS hostnames. No key. |
| **Wayback Machine** | Recovers historic URLs and long-dead subdomains for a domain from the Internet Archive CDX index. The classic "what used to be here" pivot. No key. |
| **Wikidata enrichment** | Resolves a person/company name to a Wikidata item and emits its official website, country, and related entities (employer, founders, CEO, parent company, subsidiaries) as pivotable nodes. No key. |
| **Email OSINT** | Enriches an email: Gravatar profile/avatar + linked accounts, mail provider, disposable-address flag, username pivot, and curated breach/reputation lookup links (HIBP, EmailRep, Epieos, Hunter, IntelX). No key. |
| **Maltego-style machines** | JSON-defined transform pipelines. Built-in: *Footprint L1*, *Subdomain Recon*, *Infrastructure Geo-Map* and *BR Domain Check* (Domain), *IP Profile* / *IP Geo Profile* (IP), *Email Breach Check* / *Email → Accounts* (Email), *Person Search* / *Identity Enrich* / *BR Person Dossier* (Person), *BR Company Dossier* (Organization). One click per investigation. |
| **AI analyst** | In-app Claude panel with the Claude-OSINT investigative-tradecraft skills loaded as system prompt; has read access to your graph for grounded suggestions. |
| **Multi-case workspace** | Tabbed cases, rename / delete / export. GraphML and CSV export for hand-off. |
| **Multi-select transforms** | Box-select or Ctrl-click multiple nodes; right-click runs intersection-compatible transforms across all of them. |
| **Auto-collapse Collections** | When a transform returns >10 entities of one type, they collapse into a single Collection node; expand on demand. |
| **5 graph layouts** | Force-directed (fcose), hierarchical (dagre), concentric, circle, grid — positions persist. |
| **Find-on-graph** | Ctrl/Cmd+F to jump to any entity by label or type substring. |
| **API-key vault** | Per-user secrets store (mode 0600) for SpiderFoot's keyed modules. |
| **Plugin SDK** | Drop a directory in `~/.local/share/dev.maltrace.app/plugins/` — no Rust edits — and your transforms appear at next restart. |
| **Dual-target** | Same React frontend runs as a Tauri desktop app OR a standalone headless web build via the `maltrace-server` binary. |
| **Audit trail** | Every transform invocation is logged (case_id, transform_id, duration, status, error, new_entity_count) for performance + forensic review. |

---

## Documentation

| Doc | For |
|---|---|
| **[USER_GUIDE.md](docs/USER_GUIDE.md)** | Every feature, panel, shortcut, env var — the complete reference. |
| **[INVESTIGATION_GUIDE.md](docs/INVESTIGATION_GUIDE.md)** | Walk-through of a realistic investigation, end to end. |
| **[PLUGIN_SDK.md](docs/PLUGIN_SDK.md)** | Public contract for writing your own transform plugin. Minimal viable plugin is ~40 lines. |
| **[DEVELOPMENT.md](docs/DEVELOPMENT.md)** | Building maltrace itself: prereqs, layout, dev workflow. |

---

## Quick start

```bash
# Prereqs: Rust, Node 20+, Python 3.11–3.13. The setup script will
# clone the sibling spiderfoot/ checkout if it's missing.

git clone https://github.com/<you>/maltrace.git
cd maltrace
./scripts/setup.sh     # bootstraps npm + all three sidecar venvs (idempotent)
./scripts/run.sh       # launches the desktop app
```

That's it. To enable the AI panel, drop your key into the auto-generated
`.env` (`ANTHROPIC_API_KEY=sk-ant-…`) and relaunch.

Headless web build: `./scripts/run.sh --server` (axum on http://127.0.0.1:1421).

See [DEVELOPMENT.md](docs/DEVELOPMENT.md) if you'd rather set things up
by hand, and [sidecars/face-search/SETUP.md](sidecars/face-search/SETUP.md)
for the InsightFace caveats (use `uv` if you have it — `setup.sh` picks
it up automatically).

---

## Architecture

```
┌──────────────────────────────────────────────┐
│ React frontend (Cytoscape.js)                │
│   Graph canvas · inspector · transform menu  │
│   AI panel · machines · finder · case tabs   │
└────────────────┬─────────────────────────────┘
                 │ Tauri IPC  /  fetch /api/*  (auto-detected)
┌────────────────┴─────────────────────────────┐
│ Rust core (transport-agnostic in core.rs)    │
│   Graph state · ontology · plugin registry   │
│   SQLite (cases / entities / edges / runs)   │
│   Secrets store · machine runner · AI proxy  │
└────────────────┬─────────────────────────────┘
                 │ subprocess + JSON (sidecar protocol v1)
┌────────────────┴─────────────────────────────┐
│ Python sidecars (plug-ins, plugin.toml)      │
│   spiderfoot-adapter · face-search · brazuca │
│   username-search · crypto-osint · wayback   │
│   cert-transparency · net-intel · wikidata   │
│   email-intel · your own plugins …           │
└──────────────────────────────────────────────┘
```

The Rust core is transport-agnostic, so the same React frontend talks
to either Tauri's IPC (desktop binary) or the standalone
`maltrace-server` binary over HTTP. Plugins are discovered by scanning
`plugin.toml` manifests at startup; the registry handles dispatch.

---

## Built on

Three MIT-licensed upstream projects, integrated rather than re-invented:

- **[SpiderFoot](https://github.com/smicallef/spiderfoot)** — transform engine; 230+ modules wrapped.
- **[Claude-OSINT](https://github.com/Claude-OSINT)** — investigative-tradecraft skill primers loaded into the AI panel.
- **[osint-brazuca](https://github.com/osintbrazuca/osint-brazuca)** — Brazilian resource catalogue, surfaced as the brazuca recommender.

The face-search sidecar is first-party maltrace code (InsightFace + reverse-image-search backends).

---

## License

[MIT](LICENSE).
