# Running an Investigation in maltrace

A guided walk-through of a realistic OSINT investigation, using the
features documented in [USER_GUIDE.md](USER_GUIDE.md). The scenario is
fictional but the techniques are real.

> **Scenario.** A phishing email arrives at your company claiming to
> be a Brazilian bank. The "Click here to verify your account" link
> points at `https://banco-do-brasil.example` (made up — substitute
> your favourite test domain). You're asked to determine who runs the
> domain, whether it's tied to known infrastructure, and what other
> assets they own.

The walk-through assumes maltrace is installed and at least the
SpiderFoot and Brazuca sidecars are working. If you want the AI
analyst, also set `ANTHROPIC_API_KEY`.

---

## 1. Create a dedicated case

Click the **+** on the case-tab strip in the topbar to make a fresh
case. Double-click its tab to rename it — call it something memorable
like *"phishing 2026-03-18"*.

Why bother: separating investigations keeps the graph readable, makes
later GraphML/CSV export targeted, and means deleting the case is a
one-click reset if you need to start over.

---

## 2. Drop the seed entity

In the topbar add-entity form, pick **Domain**, paste
`banco-do-brasil.example`, and click **Add**. A green node appears.

> **Tip.** If you have multiple suspicious URLs, paste them all in.
> The graph will pivot from any of them — and you can compare what
> ends up shared between them.

---

## 3. Run a baseline footprint with a machine

Select the Domain node. Click **Machines** in the topbar → **Footprint
L1** → **Run**.

What just happened: a three-step machine ran WHOIS, DNS resolution, and
subdomain enumeration in sequence. The machine panel shows per-step
results — how many entities each step produced and any failures.

Expect to see:

- A grey **WHOIS** node attached to the domain — registrar, registrant,
  dates.
- One or more amber **IP** nodes — the addresses the domain resolves to.
- Possibly a Collection node if there are >10 subdomains (see §6 below).

---

## 4. Inspect what came back

Click the WHOIS node. The inspector pane shows the full WHOIS payload —
registrar, registrant, name servers, creation/expiration dates,
contact emails. Things that are interesting to look for:

- **Registration date** within the last 30 days — fresh domains are
  more suspicious for phishing.
- **Privacy proxy registrant** — common but not damning; combine with
  other signals.
- **Name servers pointing at a known bulletproof host**.
- **Registrant email reuse** — copy it; you'll pivot on it in a moment.

---

## 5. Pivot from an IP

Click one of the IP nodes. The inspector now shows compatible
transforms — anything that consumes `IP_ADDRESS`, grouped by plugin
source. Useful pivots from an IP:

- `sfp_ipinfo` — geolocation, ASN owner.
- `sfp_abuseipdb` — known-bad reputation.
- `sfp_ripe` — RIR allocation history.
- `sfp_alienvault` — threat-intel correlation.
- `sfp_reversednsdb` — other domains hosted at the same address.

Many of these need API keys. See [§8 Secrets](#8-set-up-api-keys-as-needed).

Or run the **IP Profile** machine — same set in one click.

---

## 6. Handle the Collection explosion

If reverse-DNS came back with 47 other domains on the same IP, you
won't see 47 nodes — you'll see a dashed-rectangle **Collection** node
labelled *"47 Domain"*.

Click it. The inspector shows a preview list. Two options:

- **Expand** — materialise all 47 as individual nodes. Useful when
  you're going to triage them anyway.
- **Don't expand** — keep the graph readable, drill into the preview,
  manually add the interesting ones as fresh entities.

A common pattern: expand, then immediately re-run **Footprint L1** as
a batch on the whole 47. Box-select all of them on the canvas, then
right-click → run the relevant transforms across the multi-selection.
Compatible transforms in the multi-select menu only include those
that accept *every* selected entity — so the menu is a clean
"what can I do across all of these".

---

## 7. Pick a layout that fits

By now you have dozens of nodes and the default ring layout may look
chaotic. Open the floating controls (bottom-right of the canvas), hit
the **Layout** dropdown:

- **Organic** (fcose) — force-directed; the workhorse for general
  exploration. Try this first.
- **Hierarchical** (dagre) — when you want to show the chain *seed →
  WHOIS → IP → other domains*. Good for screenshots in reports.
- **Concentric** — useful when one entity is the centre of the
  investigation (the seed) and importance falls off with distance.

Picking a layout re-runs it once and persists every position. Close the
app and the graph keeps that shape next session.

---

## 8. Set up API keys as needed

Many high-signal SpiderFoot modules need keys. The Plugins modal
(click "**N transforms**" in the topbar status area) shows you
**not runnable** badges if a plugin's venv is missing entirely, but a
runnable plugin can still fail per-transform because a specific
module's key is unset.

For now, set keys via the backend API (a UI manager is on the polish
list). From a script:

```js
// for example, in a browser console pointed at the maltrace-server
// or via your own Tauri-side invocation
await api.setSecret("hibp_api_key", "YOUR-KEY-HERE");
```

Common ones worth setting:

| Key | Used by |
|---|---|
| `hibp_api_key` | `sfp_haveibeenpwned` |
| `shodan_api_key` | `sfp_shodan` |
| `virustotal_api_key` | `sfp_virustotal` |
| `hunter_api_key` | `sfp_hunter` |
| `abuseipdb_api_key` | `sfp_abuseipdb` |
| `intelx_api_key` | `sfp_intelx` |

Keys are stored in `~/.local/share/dev.maltrace.app/secrets.json` (mode
0600 on Unix); the sidecar receives them on every invocation. Restart
not needed.

---

## 9. Use the Brazuca recommender for BR context

Brazilian context shows up a lot in this scenario. Right-click the
domain → look for the **brazuca** plugin source in the transform list
→ run **brazuca_resources**.

That single transform spits out a cluster of URL nodes — *Registro.br
WHOIS*, *NIC.br statistics*, *HudsonRock breach lookup*, *Google
site:gov.br search*. Click any one of them, then click the **Open**
icon in the inspector header to launch it in your default browser.

If you'd pivoted from a Person entity labelled with a CPF format
(`123.456.789-09`), brazuca would have given you *Receita Federal CPF
Consulta*, *TSE Eleitoral*, *JusBrasil*, *Casa dos Dados*, etc.
Pattern-matched by the entity's label, not just its type.

---

## 10. Trace a person back from an email

Suppose the WHOIS gave you `admin@suspicious-host.example`. Add it
manually as an Email entity, or it may have appeared automatically.
Run the **Email Breach Check** machine:

- `sfp_haveibeenpwned` — credential leaks.
- `sfp_gravatar` — sometimes ties the email to a real photo.
- Cross-platform account enumeration.

If a Gravatar photo comes back, you now have an **Image** entity. With
face-search set up, right-click the image → `face_search_yandex` to
search the web for other appearances of that face. Each match becomes
a URL node with a similarity score in the `extra` payload.

---

## 11. Ask the analyst

Click **Analyst** in the topbar. The AI panel opens. With your phishing
domain selected, ask:

> "Given the WHOIS, the IPs, and the related domains we've found, what
> does this infrastructure pattern look like? What's worth pivoting on
> next?"

The panel ships your selected entity, an entity count + type mix
summary, and recent entities to Claude. Claude has the Claude-OSINT
investigative tradecraft loaded as a system prompt, so it actually
knows what good pivots look like for this kind of investigation.

Watch the token usage strip at the bottom of the panel. After the
first turn you should see `cached <large> (>90%)` — that means prompt
caching is working and follow-ups are ~10× cheaper than the first
turn. If `cached 0`, something invalidated the cache; check
`MALTRACE_AI_MODEL` hasn't changed mid-session.

Things the analyst is particularly good at, given the skills loaded:

- Spotting infrastructure-reuse patterns ("this looks like the same
  operator as X")
- Suggesting which API-key-gated module would help most ("you don't
  have Shodan results yet — that would tell you about open services on
  these IPs")
- Drafting the conclusion paragraph for your report

---

## 12. Find your way around with `Ctrl+F`

Once the graph is dense, **Ctrl/Cmd + F** opens the finder. Type any
substring (`.br`, `gmail`, `203.0.113`, a CPF, anything). Hit Enter —
the canvas pans and zooms to that entity and selects it.

This pairs well with the AI suggesting *"check the IP 203.0.113.42"* —
hit Ctrl-F, type `113.42`, Enter, look at it.

---

## 13. Snapshot for hand-off

When you're ready to hand the case to a teammate or archive it:

- **Right-click the case tab → Export** — downloads a `.graphml` file.
  They can open it in Maltego, Gephi, yEd, or any GraphML-aware tool
  without needing maltrace installed.
- **CSV export** — flat list of entities for spreadsheet workflows.
  Useful when someone wants "give me every IP we found".
- For shipping the *full* picture (entities + edges + transform run
  history + payloads), the SQLite database at
  `~/.local/share/dev.maltrace.app/maltrace.sqlite` is the source of
  truth. Copy it; `sqlite3 maltrace.sqlite` to query it.

---

## 14. After-action: review what you ran

Every transform invocation is logged in the `transform_runs` table:
which transform, source entity, started_at, duration_ms, status,
new_entity_count, error_message, stderr_excerpt.

There isn't a UI panel for it yet (next polish), but the data is
queryable via the `list_transform_runs(case_id, limit)` backend
command, or directly with `sqlite3`:

```bash
sqlite3 ~/.local/share/dev.maltrace.app/maltrace.sqlite \
  "SELECT transform_id, source_label, status, duration_ms, new_entity_count
   FROM transform_runs WHERE case_id = 1 ORDER BY started_at DESC LIMIT 30;"
```

Useful for spotting slow transforms (worth replacing) and chronic
failures (key missing, target rate-limiting you).

---

## Investigation patterns by entity type

A quick reference for "I have this kind of entity — what next".

### Domain

1. WHOIS — `sfp_whois` or **Footprint L1** machine.
2. DNS resolution + subdomain enumeration — same machine.
3. SSL certificate inspection — `sfp_sslcert` (cross-references for
   shared cert / SAN).
4. Threat intel — `sfp_alienvault`, `sfp_abusech`, `sfp_phishtank`.
5. Brazuca, if `.br` — adds Registro.br + Brazilian breach / search
   resources.

### IP

1. **IP Profile** machine — ownership, geolocation, ASN, reverse-DNS.
2. Open ports / banners — `sfp_shodan` (needs key).
3. Threat intel — `sfp_abuseipdb`, `sfp_alienvault`.
4. Co-hosted domains — `sfp_reversednsdb` to find what else is on this
   address.

### Email

1. **Email Breach Check** machine.
2. Profile photo — `sfp_gravatar`.
3. Username variants — strip the `@domain` and pivot to a Username
   entity, then `sfp_account_enumerator`.
4. Disposable / role detection — `sfp_emaildiscovery`.

### Person / Username

1. Account enumeration across socials — `sfp_account_enumerator`.
2. Search engines / paste sites — `sfp_searchengine`, `sfp_pastebin`.
3. Brazuca if CPF format — gives you Receita Federal, TSE, JusBrasil.

### Image / Face

1. `face_detect` first — confirms there's a face, gives you bounding
   box and quality score. No network.
2. `face_search_yandex` for free reverse search.
3. `face_search_bing` if you have an Azure Visual Search key.
4. `face_search_facecheck` for serious investigations (needs API key).
5. `face_search_all` to run every available backend and rank by
   cosine similarity — slowest but most thorough.

---

## Habits worth forming

- **Always start a new case per investigation.** Cross-contamination
  between investigations makes the AI's graph-context summaries
  useless.
- **Run a machine first, ad-hoc transforms second.** A machine guarantees
  you have the baseline before you go hunting; ad-hoc work is faster
  once you know where the interesting threads are.
- **Re-layout when the graph gets dense.** Five clicks; saves five
  minutes of squinting.
- **Use the AI panel as a sounding board, not an oracle.** It's
  excellent at suggesting pivots and spotting patterns. Don't let it
  draft the conclusion from thin air — feed it the WHOIS payload, the
  IP details, the breach hits, then ask for synthesis.
- **Export GraphML before you delete a case.** Cheap insurance.
- **Set API keys early.** A `sfp_shodan` that silently no-ops because
  the key isn't set will waste five minutes before you realise.

---

## See also

- [USER_GUIDE.md](USER_GUIDE.md) — feature reference.
- [PLUGIN_SDK.md](PLUGIN_SDK.md) — write your own transforms when a
  specific lookup is missing.
- [DEVELOPMENT.md](DEVELOPMENT.md) — building maltrace itself.
