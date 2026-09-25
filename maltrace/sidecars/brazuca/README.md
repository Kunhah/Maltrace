# brazuca

Third maltrace sidecar. Two flavours of transform:

1. **`brazuca_resources`** — curated catalogue of Brazilian OSINT portals,
   surfaced as clickable URL entities. Offline.
2. **Live scrapers** against free, no-key public APIs:
   - `brazuca_brasilapi_cnpj` — Receita-grade CNPJ data (razão social,
     atividade, endereço, telefones, e-mail, **quadro de sócios**).
   - `brazuca_brasilapi_cep` — CEP → endereço + coordinates.
   - `brazuca_brasilapi_ddd` — area code → state + cities.
   - `brazuca_brasilapi_bank` — 3-digit bank code → bank name + ISPB.
   - `brazuca_viacep` — alternative CEP → endereço.
   - `brazuca_hudson_rock_email|domain|username` — Hudson Rock Cavalier's
     free infostealer/credential-leak API.

## What `brazuca_resources` does

Right-click any Person / Organization / Domain / Email / Phone / IP entity
→ run it → one URL node appears per matching Brazilian resource (Receita
Federal, TSE, JusBrasil, Registro.br WHOIS, Anatel, CVM, COAF, BCB,
tribunais estaduais, etc.). Each URL carries title, description, and
category as structured metadata in the inspector.

## Why catalogue *and* scrapers?

Most authoritative Brazilian OSINT portals (Receita CPF/CNPJ, TSE, gov.br
courts) require CAPTCHA or login and can't be safely automated. For
those, the catalogue stops at "here's the right URL to visit". But where
a public, key-less API exists (BrasilAPI, ViaCEP, Hudson Rock's free
endpoints), the scraper version pulls the actual data into the graph
without leaving the app.

## Gating

Each resource has an `applies_to` rule. Examples:

| Rule | Matches |
|---|---|
| `"Person"` | any Person entity |
| `"Person:cpf"` | Person whose label looks like `XXX.XXX.XXX-XX` |
| `"Organization:cnpj"` | Organization whose label looks like `XX.XXX.XXX/XXXX-XX` |
| `"PhoneNumber:phone_br"` | Phone matching Brazilian formats |
| `"Domain:.br"` | Domain ending in `.br` |
| `"*"` | always |

Patterns live in `adapter.py`'s `PATTERNS` dict. Adding a new pattern is
one regex + a usage in `applies_to`.

## Adding resources

Edit the `RESOURCES` list in `adapter.py`. Each entry:

```python
{
    "id": "unique_snake_case",
    "title": "Display name",
    "url": "https://...",
    "description": "What you'll find here / any caveats (CAPTCHA, login).",
    "applies_to": ["Person:cpf"],
    "category": "Pessoas",
}
```

When the catalogue grows past ~50 entries, move it to `resources.yaml`
(pyyaml is cheap).

## Requirements

Pure Python stdlib — no extra deps (`urllib` for HTTP). Still uses a
venv for consistency with the other sidecars.

```bash
python3 -m venv .venv
# no `pip install` needed
.venv/bin/python adapter.py list-transforms
.venv/bin/python adapter.py run --module brazuca_resources \
  --event-type HUMAN_NAME --data "123.456.789-09"
.venv/bin/python adapter.py run --module brazuca_brasilapi_cep \
  --event-type PHYSICAL_ADDRESS --data "01310-100"
```

## Why one catalogue transform vs many

For the catalogue side, each resource could have been its own transform —
but right-clicking a Person and seeing 20 separate `brazuca_*` entries is
noisy. One transform that returns every matching URL is cleaner UX. The
live scrapers are split per data source so the inspector can show
provenance per emitted entity.
