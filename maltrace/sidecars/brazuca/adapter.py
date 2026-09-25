"""
maltrace ↔ brazuca adapter.

Two flavours of transform live here:

  1. brazuca_resources       — curated catalogue of Brazilian OSINT portals,
                                surfaced as openable URL entities. Offline.
  2. brazuca_brasilapi_*     — live lookups against BrasilAPI (CNPJ, CEP,
                                DDD, bank). Free, no key.
  3. brazuca_viacep          — live CEP → address lookup via ViaCEP.
  4. brazuca_hudson_rock_*   — Hudson Rock Cavalier free OSINT API:
                                infostealer/credential exposure for emails,
                                domains, and usernames. No key required.
  5. brazuca_cpf_validate    — offline CPF check-digit validation + fiscal
                                region inference. No network.
  6. brazuca_cnpj_validate   — offline CNPJ check-digit validation +
                                matriz/filial detection. No network.
  7. brazuca_registrobr      — .br domain status via BrasilAPI (no key).
  8. brazuca_camara_deputado — Câmara dos Deputados open data, by name
                                (PEP screening). No key.
  9. brazuca_querido_diario  — full-text search across municipal official
                                gazettes (Querido Diário / OKBR). No key.

One-shot JSON protocol shared with the other sidecars:

    python adapter.py list-transforms
    python adapter.py run --module <id> --event-type <SF_EVENT> --data <LABEL>

Stdout: JSON. Stderr: diagnostics. Pure stdlib (urllib for HTTP) — no extra
deps. Each subprocess is one-shot so we don't bother with connection reuse.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import traceback
import urllib.error
import urllib.parse
import urllib.request
from typing import Any, List, Optional


# ──────────────────────────────────────────────────────────────────────
#  SpiderFoot event → canonical entity-type map.
#
#  Embedded here (not imported from maltrace) because sidecars stay
#  independent. Only the events we actually care about are listed; everything
#  else falls through to "Generic" and applies_to rules referencing canonical
#  types simply won't match (except wildcards).
# ──────────────────────────────────────────────────────────────────────

SF_TO_CANONICAL = {
    # Person
    "HUMAN_NAME": "Person",
    "JOB_TITLE": "Person",
    # Organization
    "COMPANY_NAME": "Organization",
    "AFFILIATE_COMPANY_NAME": "Organization",
    "LEI": "Organization",
    # Domain
    "DOMAIN_NAME": "Domain",
    "INTERNET_NAME": "Domain",
    "AFFILIATE_DOMAIN_NAME": "Domain",
    "AFFILIATE_INTERNET_NAME": "Domain",
    "SIMILARDOMAIN": "Domain",
    # Email
    "EMAILADDR": "EmailAddress",
    "AFFILIATE_EMAILADDR": "EmailAddress",
    # Phone
    "PHONE_NUMBER": "PhoneNumber",
    # IP
    "IP_ADDRESS": "IpAddress",
    "IPV6_ADDRESS": "IpAddress",
    # Username
    "USERNAME": "Username",
    "ACCOUNT_EXTERNAL_OWNED": "Username",
    # Address / geo
    "PHYSICAL_ADDRESS": "Address",
}


# ──────────────────────────────────────────────────────────────────────
#  Pattern matchers used by `applies_to: "Type:pattern"` rules.
#
#  Patterns are matched against the entity label. Add new ones here and
#  reference them from a resource's applies_to. Patterns use `re.search` so
#  they need not match the whole string unless anchored.
# ──────────────────────────────────────────────────────────────────────

PATTERNS = {
    # Brazilian CPF: XXX.XXX.XXX-XX (with/without separators).
    "cpf": re.compile(r"^\d{3}\.?\d{3}\.?\d{3}-?\d{2}$"),
    # Brazilian CNPJ: XX.XXX.XXX/XXXX-XX.
    "cnpj": re.compile(r"^\d{2}\.?\d{3}\.?\d{3}/?\d{4}-?\d{2}$"),
    # Brazilian phone with optional +55 country code.
    "phone_br": re.compile(r"^(?:\+?55\s?)?\(?\d{2}\)?\s?\d{4,5}-?\d{4}$"),
    # Domain in the .br TLD (case-insensitive).
    ".br": re.compile(r"\.br$", re.IGNORECASE),
    # Email in a .br domain.
    "email_br": re.compile(r"@[\w.-]+\.br$", re.IGNORECASE),
    # Brazilian CEP (postal code) — 8 digits, optional dash.
    "cep": re.compile(r"\b\d{5}-?\d{3}\b"),
    # Vehicle plate — old (ABC-1234) or Mercosul (ABC1D23).
    "placa": re.compile(r"^[A-Za-z]{3}-?\d[A-Za-z0-9]\d{2}$"),
    # Voter-registration number (título de eleitor) — 12 digits.
    "titulo": re.compile(r"^\d{12}$"),
}


# ──────────────────────────────────────────────────────────────────────
#  Curated catalogue.
#
#  Source: hand-picked from osint-brazuca's README + common professional BR
#  OSINT tooling. Keep additions to high-quality, broadly useful sources —
#  per-state niche portals can hide behind better grouping later.
# ──────────────────────────────────────────────────────────────────────

RESOURCES: List[dict] = [
    # ─── Pessoas / CPF ─────────────────────────────────────
    {
        "id": "rfb_cpf",
        "title": "Receita Federal — Consulta CPF",
        "url": "https://servicos.receita.fazenda.gov.br/Servicos/CPF/ConsultaSituacao/ConsultaPublica.asp",
        "description": "Authoritative CPF status check (CAPTCHA + DOB required).",
        "applies_to": ["Person:cpf"],
        "category": "Pessoas",
    },
    {
        "id": "tse_eleitor",
        "title": "TSE — Consulta de Situação Eleitoral",
        "url": "https://www.tse.jus.br/eleitor/servicos/titulo-de-eleitor/consulta-por-titulo-nome-ou-cpf",
        "description": "Voter registration lookup by título / nome / CPF.",
        "applies_to": ["Person", "Person:cpf"],
        "category": "Pessoas",
    },
    {
        "id": "consultasocio_pessoa",
        "title": "ConsultaSocio — busca por nome",
        "url": "https://www.consultasocio.com/",
        "description": "Find companies a person holds shares in (by name or CPF).",
        "applies_to": ["Person"],
        "category": "Pessoas / Empresas",
    },
    {
        "id": "yantra_pessoas",
        "title": "Yantra — busca aberta de pessoas",
        "url": "https://yantra.com.br/",
        "description": "Aggregator (CPF, nome, telefone). Free tier limited.",
        "applies_to": ["Person", "Person:cpf"],
        "category": "Pessoas",
    },
    {
        "id": "escavador_pessoas",
        "title": "Escavador — perfil público",
        "url": "https://www.escavador.com/sobre",
        "description": "People search: judicial processes, mentions, related entities.",
        "applies_to": ["Person"],
        "category": "Pessoas",
    },
    {
        "id": "siglas_inss_meu",
        "title": "Meu INSS — Extrato Previdenciário",
        "url": "https://meu.inss.gov.br/",
        "description": "Authoritative social-security history (requires login as the target).",
        "applies_to": ["Person:cpf"],
        "category": "Pessoas",
    },

    # ─── Empresas / CNPJ ──────────────────────────────────
    {
        "id": "rfb_cnpj",
        "title": "Receita Federal — Consulta CNPJ",
        "url": "https://solucoes.receita.fazenda.gov.br/Servicos/cnpjreva/cnpjreva_solicitacao.asp",
        "description": "Authoritative CNPJ situation / sócios / atividades. CAPTCHA.",
        "applies_to": ["Organization", "Organization:cnpj"],
        "category": "Empresas",
    },
    {
        "id": "casadosdados",
        "title": "Casa dos Dados — CNPJ search",
        "url": "https://casadosdados.com.br/",
        "description": "Free CNPJ + partners + addresses lookup with related-companies graph.",
        "applies_to": ["Organization", "Organization:cnpj"],
        "category": "Empresas",
    },
    {
        "id": "empresometro",
        "title": "Empresômetro",
        "url": "https://empresometro.com.br/",
        "description": "Company database with financial metrics; paywall for deeper data.",
        "applies_to": ["Organization", "Organization:cnpj"],
        "category": "Empresas",
    },
    {
        "id": "brasil_io_socios",
        "title": "Brasil.IO — Sócios de Empresas",
        "url": "https://brasil.io/dataset/socios-brasil/socios/",
        "description": "Open dataset of Brazilian companies and their partners (CPF/CNPJ).",
        "applies_to": ["Organization", "Person", "Organization:cnpj", "Person:cpf"],
        "category": "Empresas",
    },
    {
        "id": "consultasocio_empresa",
        "title": "ConsultaSocio — empresa",
        "url": "https://www.consultasocio.com/",
        "description": "Partners and shareholding history for a CNPJ.",
        "applies_to": ["Organization", "Organization:cnpj"],
        "category": "Empresas",
    },
    {
        "id": "cvm_emissores",
        "title": "CVM — Sistemas de Emissores",
        "url": "https://sistemas.cvm.gov.br/?CadGeral",
        "description": "Comissão de Valores Mobiliários: filings, controllers, related parties for listed companies.",
        "applies_to": ["Organization", "Organization:cnpj"],
        "category": "Empresas",
    },
    {
        "id": "b3_empresas",
        "title": "B3 — Companhias Listadas",
        "url": "https://www.b3.com.br/pt_br/produtos-e-servicos/negociacao/renda-variavel/empresas-listadas.htm",
        "description": "Bolsa brasileira: filings, structure, shareholdings for B3-listed companies.",
        "applies_to": ["Organization"],
        "category": "Empresas",
    },

    # ─── Compliance / Sanctions ───────────────────────────
    {
        "id": "cgu_ceis",
        "title": "Portal da Transparência — CEIS (sanções)",
        "url": "https://portaldatransparencia.gov.br/sancoes/ceis",
        "description": "Cadastro de Empresas Inidôneas e Suspensas (gov-sanctioned suppliers).",
        "applies_to": ["Organization", "Organization:cnpj"],
        "category": "Compliance",
    },
    {
        "id": "cgu_cnep",
        "title": "Portal da Transparência — CNEP",
        "url": "https://portaltransparencia.gov.br/sancoes/cnep",
        "description": "Cadastro Nacional de Empresas Punidas (Lei Anticorrupção).",
        "applies_to": ["Organization", "Organization:cnpj"],
        "category": "Compliance",
    },
    {
        "id": "cgu_servidores",
        "title": "Portal da Transparência — Servidores",
        "url": "https://portaldatransparencia.gov.br/servidores",
        "description": "Federal-government civil servants by name / CPF (partial) / órgão.",
        "applies_to": ["Person"],
        "category": "Compliance",
    },
    {
        "id": "coaf_obrigados",
        "title": "COAF — Listas de Sanções",
        "url": "https://www.gov.br/coaf/pt-br/assuntos/listas-de-sancoes",
        "description": "Council on Financial Activities Control — sanction lists (UN, OFAC mirror).",
        "applies_to": ["Person", "Organization"],
        "category": "Compliance",
    },
    {
        "id": "bcb_unimpedidos",
        "title": "Banco Central — Pessoas Impedidas",
        "url": "https://www.bcb.gov.br/estabilidadefinanceira/pessoasimpedidas",
        "description": "BCB list of persons barred from banking-system management.",
        "applies_to": ["Person"],
        "category": "Compliance",
    },

    # ─── Tribunais / Justiça ──────────────────────────────
    {
        "id": "jusbrasil",
        "title": "JusBrasil — busca de processos",
        "url": "https://www.jusbrasil.com.br/",
        "description": "Cross-tribunal lawsuit search by name, CPF or CNPJ.",
        "applies_to": ["Person", "Organization"],
        "category": "Tribunais",
    },
    {
        "id": "cnj_pje",
        "title": "CNJ — PJe (busca unificada)",
        "url": "https://www.cnj.jus.br/sistemas/processo-judicial-eletronico-pje/",
        "description": "Conselho Nacional de Justiça PJe portal — entry point for federal/state courts.",
        "applies_to": ["Person", "Organization"],
        "category": "Tribunais",
    },
    {
        "id": "tjsp_esaj",
        "title": "TJSP — Consulta Processual (e-SAJ)",
        "url": "https://esaj.tjsp.jus.br/cpopg/open.do",
        "description": "Tribunal de Justiça de São Paulo — case search by name / CPF / CNPJ / process number.",
        "applies_to": ["Person", "Organization"],
        "category": "Tribunais",
    },
    {
        "id": "tjrj_consulta",
        "title": "TJRJ — Consulta Processual",
        "url": "https://www4.tjrj.jus.br/consultaProcessoWebV2/consultaProc.do",
        "description": "Tribunal de Justiça do Rio de Janeiro — process search.",
        "applies_to": ["Person", "Organization"],
        "category": "Tribunais",
    },
    {
        "id": "tjmg_consulta",
        "title": "TJMG — Consulta Processual",
        "url": "https://www.tjmg.jus.br/portal-tjmg/processos/consulta-de-processos.htm",
        "description": "Tribunal de Justiça de Minas Gerais — process search.",
        "applies_to": ["Person", "Organization"],
        "category": "Tribunais",
    },
    {
        "id": "trf_jurisprudencia",
        "title": "Justiça Federal — Jurisprudência",
        "url": "https://www.cjf.jus.br/sis-jurisprudencia/",
        "description": "Federal courts' jurisprudence search.",
        "applies_to": ["Person", "Organization"],
        "category": "Tribunais",
    },
    {
        "id": "stf_jurisprudencia",
        "title": "STF — Jurisprudência",
        "url": "https://jurisprudencia.stf.jus.br/pages/search",
        "description": "Supreme Federal Court jurisprudence search.",
        "applies_to": ["Person", "Organization"],
        "category": "Tribunais",
    },
    {
        "id": "stj_jurisprudencia",
        "title": "STJ — Jurisprudência",
        "url": "https://scon.stj.jus.br/SCON/",
        "description": "Superior Court of Justice jurisprudence search.",
        "applies_to": ["Person", "Organization"],
        "category": "Tribunais",
    },
    {
        "id": "tst_consulta",
        "title": "TST — Consulta Processual Trabalhista",
        "url": "https://consultaprocessual.tst.jus.br/consultaProcessual/consultaTstNumUnica.do",
        "description": "Labour Court (TST) process consultation by party or number.",
        "applies_to": ["Person", "Organization"],
        "category": "Tribunais",
    },

    # ─── Domínios / DNS ───────────────────────────────────
    {
        "id": "registro_br_whois",
        "title": "Registro.br — WHOIS",
        "url": "https://registro.br/tecnologia/ferramentas/whois/",
        "description": "Authoritative WHOIS for .br domains (CAPTCHA).",
        "applies_to": ["Domain:.br"],
        "category": "Domínios",
    },
    {
        "id": "registro_br_disponibilidade",
        "title": "Registro.br — Disponibilidade",
        "url": "https://registro.br/",
        "description": "Check whether a .br domain is registered.",
        "applies_to": ["Domain:.br"],
        "category": "Domínios",
    },

    # ─── Telefones ────────────────────────────────────────
    {
        "id": "anatel_consulta_operadora",
        "title": "Anatel — Consulta de Operadora",
        "url": "https://sistemas.anatel.gov.br/cmp/Default.aspx",
        "description": "Identify the operator (Vivo/Tim/Claro/…) of a Brazilian number.",
        "applies_to": ["PhoneNumber:phone_br"],
        "category": "Telefones",
    },
    {
        "id": "truecaller",
        "title": "Truecaller — reverse lookup",
        "url": "https://www.truecaller.com/",
        "description": "Crowdsourced caller-ID database; covers BR.",
        "applies_to": ["PhoneNumber", "PhoneNumber:phone_br"],
        "category": "Telefones",
    },

    # ─── Endereços / Geo ─────────────────────────────────
    {
        "id": "viacep",
        "title": "ViaCEP — busca de CEP",
        "url": "https://viacep.com.br/",
        "description": "Free CEP (postal code) lookup with REST API.",
        "applies_to": ["*"],
        "category": "Endereços",
    },
    {
        "id": "brasilapi",
        "title": "BrasilAPI — CEP / CNPJ / DDD / Bancos",
        "url": "https://brasilapi.com.br/docs",
        "description": "Free aggregated REST API for several BR public datasets (no key).",
        "applies_to": ["*"],
        "category": "Endereços",
    },

    # ─── Breach / Email ──────────────────────────────────
    {
        "id": "hudson_rock_cavalier",
        "title": "Hudson Rock — Cavalier (free)",
        "url": "https://www.hudsonrock.com/free-tools",
        "description": "Free infostealer/credential-leak lookup by email/domain/username.",
        "applies_to": ["EmailAddress", "Domain", "Domain:.br", "Username"],
        "category": "Vazamentos",
    },
    {
        "id": "haveibeenpwned",
        "title": "Have I Been Pwned",
        "url": "https://haveibeenpwned.com/",
        "description": "Check email/domain exposure in known breaches.",
        "applies_to": ["EmailAddress", "Domain"],
        "category": "Vazamentos",
    },
    {
        "id": "intelx",
        "title": "Intelligence X — leak search",
        "url": "https://intelx.io/",
        "description": "Search leaks, pastes, dark-web indexes by selector (email, domain, phone, BTC).",
        "applies_to": ["EmailAddress", "Domain", "PhoneNumber", "Person"],
        "category": "Vazamentos",
    },
    {
        "id": "dehashed",
        "title": "DeHashed",
        "url": "https://dehashed.com/",
        "description": "Aggregated breach database — paid; search by email/username/phone/IP.",
        "applies_to": ["EmailAddress", "Username", "PhoneNumber", "IpAddress"],
        "category": "Vazamentos",
    },

    # ─── Genéricos / Busca ───────────────────────────────
    {
        "id": "google_site_gov_br",
        "title": "Google — site:gov.br OR site:org.br",
        "url": "https://www.google.com/search?q=site%3Agov.br+OR+site%3Aorg.br",
        "description": "Scoped search across Brazilian gov / NGO sites. Append your query.",
        "applies_to": ["*"],
        "category": "Buscadores",
    },
    {
        "id": "diario_oficial",
        "title": "Imprensa Nacional — Diário Oficial",
        "url": "https://www.in.gov.br/consulta/-/buscar/dou",
        "description": "Full-text search across the federal Diário Oficial da União (DOU).",
        "applies_to": ["Person", "Organization"],
        "category": "Buscadores",
    },
    {
        "id": "portal_transparencia_convenios",
        "title": "Portal da Transparência — Convênios",
        "url": "https://portaldatransparencia.gov.br/convenios",
        "description": "Federal grants and agreements — payee CNPJ, value, programme.",
        "applies_to": ["Organization", "Organization:cnpj"],
        "category": "Compliance",
    },
    {
        "id": "portal_transparencia_beneficios",
        "title": "Portal da Transparência — Benefícios ao Cidadão",
        "url": "https://portaldatransparencia.gov.br/beneficios",
        "description": "Bolsa Família / Auxílio Brasil / BPC / Seguro-Defeso recipients by name or NIS.",
        "applies_to": ["Person", "Person:cpf"],
        "category": "Compliance",
    },
    {
        "id": "cgu_ceaf",
        "title": "Portal da Transparência — CEAF (expulsões)",
        "url": "https://portaldatransparencia.gov.br/sancoes/ceaf",
        "description": "Cadastro de Expulsões da Administração Federal — sacked civil servants.",
        "applies_to": ["Person"],
        "category": "Compliance",
    },
    {
        "id": "cgu_cepim",
        "title": "Portal da Transparência — CEPIM",
        "url": "https://portaldatransparencia.gov.br/sancoes/cepim",
        "description": "Entidades Privadas Sem Fins Lucrativos Impedidas (barred NGOs).",
        "applies_to": ["Organization", "Organization:cnpj"],
        "category": "Compliance",
    },

    # ─── Política / PEP ───────────────────────────────────
    {
        "id": "camara_deputados",
        "title": "Câmara dos Deputados — busca de deputados",
        "url": "https://www.camara.leg.br/deputados/quem-sao",
        "description": "Find federal deputies by name; profiles, mandates, expenses (cota parlamentar).",
        "applies_to": ["Person"],
        "category": "Política",
    },
    {
        "id": "senado_senadores",
        "title": "Senado Federal — Senadores",
        "url": "https://www25.senado.leg.br/web/senadores/em-exercicio",
        "description": "Sitting senators, mandates, and parliamentary activity.",
        "applies_to": ["Person"],
        "category": "Política",
    },
    {
        "id": "tse_divulgacand",
        "title": "TSE — DivulgaCandContas",
        "url": "https://divulgacandcontas.tse.jus.br/divulga/",
        "description": "Electoral candidates, declared assets, and campaign-donation records.",
        "applies_to": ["Person", "Organization"],
        "category": "Política",
    },
    {
        "id": "tcu_processos",
        "title": "TCU — Pesquisa de Processos / Acórdãos",
        "url": "https://pesquisa.apps.tcu.gov.br/",
        "description": "Federal audit court rulings naming people / companies / órgãos.",
        "applies_to": ["Person", "Organization"],
        "category": "Política",
    },

    # ─── Propriedade intelectual / mercado ────────────────
    {
        "id": "inpi_marcas",
        "title": "INPI — Busca de Marcas",
        "url": "https://busca.inpi.gov.br/pePI/jsp/marcas/Pesquisa_classe_basica.jsp",
        "description": "Trademark registry — find brands owned by a company / person.",
        "applies_to": ["Organization", "Person"],
        "category": "Mercado",
    },
    {
        "id": "inpi_patentes",
        "title": "INPI — Busca de Patentes",
        "url": "https://busca.inpi.gov.br/pePI/jsp/patentes/PatenteSearchBasico.jsp",
        "description": "Patent registry — inventions filed by a company / inventor.",
        "applies_to": ["Organization", "Person"],
        "category": "Mercado",
    },
    {
        "id": "cade_processos",
        "title": "CADE — Consulta Processual",
        "url": "https://sei.cade.gov.br/sei/institucional/pesquisa/processo_pesquisar.php",
        "description": "Antitrust authority — mergers, conduct cases, parties involved.",
        "applies_to": ["Organization"],
        "category": "Mercado",
    },
    {
        "id": "reclame_aqui",
        "title": "Reclame Aqui — reputação de empresa",
        "url": "https://www.reclameaqui.com.br/",
        "description": "Consumer-complaint platform; gauges a company's public reputation.",
        "applies_to": ["Organization", "Domain:.br"],
        "category": "Mercado",
    },
    {
        "id": "compras_gov",
        "title": "Compras.gov.br — Fornecedores",
        "url": "https://www.gov.br/compras/pt-br",
        "description": "Federal procurement portal — supplier history, contracts, bids.",
        "applies_to": ["Organization", "Organization:cnpj"],
        "category": "Mercado",
    },

    # ─── Meio ambiente / fiscalização ─────────────────────
    {
        "id": "ibama_embargos",
        "title": "IBAMA — Áreas Embargadas",
        "url": "https://servicos.ibama.gov.br/ctf/publico/areasembargadas/ConsultaPublicaAreasEmbargadas.php",
        "description": "Environmental embargoes by name / CPF / CNPJ.",
        "applies_to": ["Person", "Organization"],
        "category": "Fiscalização",
    },
    {
        "id": "ibama_autuacoes",
        "title": "IBAMA — Autos de Infração",
        "url": "https://servicos.ibama.gov.br/ctf/publico/consultaautosinfracao/index.php",
        "description": "Environmental fines / infraction notices by offender.",
        "applies_to": ["Person", "Organization"],
        "category": "Fiscalização",
    },
    {
        "id": "mte_trabalho_escravo",
        "title": "MTE — Cadastro de Trabalho Escravo (Lista Suja)",
        "url": "https://www.gov.br/trabalho-e-emprego/pt-br",
        "description": "Employers on the slave-labour 'dirty list'.",
        "applies_to": ["Person", "Organization"],
        "category": "Fiscalização",
    },

    # ─── Veículos ─────────────────────────────────────────
    {
        "id": "sinesp_cidadao",
        "title": "SINESP Cidadão — consulta de veículo",
        "url": "https://www.gov.br/pt-br/servicos/consultar-situacao-de-veiculo",
        "description": "Vehicle status / restrictions by plate (placa).",
        "applies_to": ["PhoneNumber:placa", "Person:placa", "Organization:placa", "*"],
        "category": "Veículos",
    },

    # ─── Cadastro estadual / cartórios ────────────────────
    {
        "id": "sintegra",
        "title": "SINTEGRA — Inscrição Estadual",
        "url": "http://www.sintegra.gov.br/",
        "description": "State-tax (ICMS) registration lookup; pick the company's UF.",
        "applies_to": ["Organization", "Organization:cnpj"],
        "category": "Empresas",
    },
    {
        "id": "censec",
        "title": "CENSEC — Central Notarial (CESDI/CEP)",
        "url": "https://censec.org.br/",
        "description": "Notary central: wills, powers of attorney, deeds by CPF/CNPJ.",
        "applies_to": ["Person", "Organization", "Person:cpf", "Organization:cnpj"],
        "category": "Cartórios",
    },
    {
        "id": "registradores_imoveis",
        "title": "Registradores — Penhora Online / Imóveis",
        "url": "https://www.registradores.org.br/",
        "description": "Property-registry central (ONR) — real-estate ownership lookups.",
        "applies_to": ["Person", "Organization"],
        "category": "Cartórios",
    },

    # ─── Jornalismo / diários ─────────────────────────────
    {
        "id": "querido_diario",
        "title": "Querido Diário — diários oficiais municipais",
        "url": "https://queridodiario.ok.org.br/",
        "description": "Full-text search across thousands of municipal official gazettes (Open Knowledge Brasil).",
        "applies_to": ["Person", "Organization"],
        "category": "Buscadores",
    },
    {
        "id": "jota",
        "title": "JOTA — jurídico / regulatório",
        "url": "https://www.jota.info/",
        "description": "Legal / regulatory news coverage; useful for entity context.",
        "applies_to": ["Person", "Organization"],
        "category": "Buscadores",
    },
]


# ──────────────────────────────────────────────────────────────────────
#  Transform manifest
# ──────────────────────────────────────────────────────────────────────

URL_CONSUMES = sorted(SF_TO_CANONICAL.keys())

TRANSFORMS = [
    {
        "id": "brazuca_resources",
        "name": "Brazuca — recursos BR",
        "description": (
            "Surface curated Brazilian OSINT portals relevant to the selected "
            "entity (Receita Federal, TSE, JusBrasil, Registro.br, Anatel, "
            "CVM, COAF, tribunais estaduais…)."
        ),
        "categories": ["Brazil", "Reference"],
        "group": [],
        "consumes": URL_CONSUMES,
        "produces": ["URL"],
    },
    {
        "id": "brazuca_brasilapi_cnpj",
        "name": "BrasilAPI — CNPJ (live)",
        "description": (
            "Live BrasilAPI lookup for a CNPJ. Emits company name, address, "
            "phones, emails, registered activities and partners. Free, no key."
        ),
        "categories": ["Brazil", "Lookup"],
        "group": [],
        "consumes": ["COMPANY_NAME", "AFFILIATE_COMPANY_NAME", "LEI", "RAW_RIR_DATA"],
        "produces": [
            "COMPANY_NAME",
            "PHYSICAL_ADDRESS",
            "PHONE_NUMBER",
            "EMAILADDR",
            "HUMAN_NAME",
            "RAW_RIR_DATA",
        ],
    },
    {
        "id": "brazuca_brasilapi_cep",
        "name": "BrasilAPI — CEP (live)",
        "description": (
            "Resolve a Brazilian CEP (postal code) to PHYSICAL_ADDRESS / "
            "COUNTRY_NAME / GEOINFO via BrasilAPI v2 (no key)."
        ),
        "categories": ["Brazil", "Lookup"],
        "group": [],
        "consumes": ["PHYSICAL_ADDRESS", "GEOINFO"],
        "produces": ["PHYSICAL_ADDRESS", "COUNTRY_NAME", "GEOINFO", "PHYSICAL_COORDINATES"],
    },
    {
        "id": "brazuca_brasilapi_ddd",
        "name": "BrasilAPI — DDD (área telefônica)",
        "description": (
            "Given a Brazilian phone number, derive the DDD and emit the "
            "associated state and cities via BrasilAPI."
        ),
        "categories": ["Brazil", "Lookup"],
        "group": [],
        "consumes": ["PHONE_NUMBER"],
        "produces": ["GEOINFO", "COUNTRY_NAME", "PHYSICAL_ADDRESS"],
    },
    {
        "id": "brazuca_brasilapi_bank",
        "name": "BrasilAPI — Banco (por ISPB/code)",
        "description": (
            "Resolve a Brazilian bank code (COMPASS / 3-digit) to its full "
            "name and ISPB via BrasilAPI."
        ),
        "categories": ["Brazil", "Lookup"],
        "group": [],
        "consumes": ["COMPANY_NAME"],
        "produces": ["COMPANY_NAME", "RAW_RIR_DATA"],
    },
    {
        "id": "brazuca_viacep",
        "name": "ViaCEP — CEP → endereço",
        "description": "Resolve a CEP (postal code) to a Brazilian address via ViaCEP.",
        "categories": ["Brazil", "Lookup"],
        "group": [],
        "consumes": ["PHYSICAL_ADDRESS", "GEOINFO"],
        "produces": ["PHYSICAL_ADDRESS", "COUNTRY_NAME", "GEOINFO"],
    },
    {
        "id": "brazuca_hudson_rock_email",
        "name": "Hudson Rock — Cavalier (email)",
        "description": (
            "Free Hudson Rock Cavalier API: check whether an email shows up "
            "in stealer-log infections."
        ),
        "categories": ["Brazil", "Leaks"],
        "group": [],
        "consumes": ["EMAILADDR", "AFFILIATE_EMAILADDR"],
        "produces": ["EMAILADDR_COMPROMISED", "RAW_RIR_DATA"],
    },
    {
        "id": "brazuca_hudson_rock_domain",
        "name": "Hudson Rock — Cavalier (domain)",
        "description": (
            "Free Hudson Rock Cavalier API: summary of stealer-log infections "
            "touching a given corporate / consumer domain."
        ),
        "categories": ["Brazil", "Leaks"],
        "group": [],
        "consumes": ["DOMAIN_NAME", "INTERNET_NAME"],
        "produces": ["RAW_RIR_DATA", "EMAILADDR_COMPROMISED"],
    },
    {
        "id": "brazuca_hudson_rock_username",
        "name": "Hudson Rock — Cavalier (username)",
        "description": (
            "Free Hudson Rock Cavalier API: check whether a username "
            "appears in stealer-log infections."
        ),
        "categories": ["Brazil", "Leaks"],
        "group": [],
        "consumes": ["USERNAME", "ACCOUNT_EXTERNAL_OWNED"],
        "produces": ["RAW_RIR_DATA"],
    },
    {
        "id": "brazuca_cpf_validate",
        "name": "CPF — validação + região fiscal (offline)",
        "description": (
            "Validate a CPF's check digits (mod-11) offline and infer the "
            "Receita Federal fiscal region / states of issue from the 9th "
            "digit. No network, no key."
        ),
        "categories": ["Brazil", "Detect"],
        "group": [],
        "consumes": ["HUMAN_NAME", "RAW_RIR_DATA", "USERNAME", "ACCOUNT_EXTERNAL_OWNED"],
        "produces": ["HUMAN_NAME", "GEOINFO", "RAW_RIR_DATA"],
    },
    {
        "id": "brazuca_cnpj_validate",
        "name": "CNPJ — validação + matriz/filial (offline)",
        "description": (
            "Validate a CNPJ's check digits (mod-11) offline and report "
            "whether it is the matriz (0001) or a filial branch. No network."
        ),
        "categories": ["Brazil", "Detect"],
        "group": [],
        "consumes": ["COMPANY_NAME", "AFFILIATE_COMPANY_NAME", "LEI", "RAW_RIR_DATA"],
        "produces": ["COMPANY_NAME", "RAW_RIR_DATA"],
    },
    {
        "id": "brazuca_registrobr",
        "name": "Registro.br — status do domínio .br (live)",
        "description": (
            "Query BrasilAPI's registro.br endpoint for a .br domain: "
            "registration status, expiry date, nameservers and (if free) "
            "suggestions. No key."
        ),
        "categories": ["Brazil", "Lookup"],
        "group": [],
        "consumes": ["DOMAIN_NAME", "INTERNET_NAME", "AFFILIATE_DOMAIN_NAME"],
        "produces": ["RAW_RIR_DATA", "PROVIDER_DNS", "DOMAIN_REGISTRAR"],
    },
    {
        "id": "brazuca_camara_deputado",
        "name": "Câmara dos Deputados — busca por nome (live)",
        "description": (
            "Search the Câmara dos Deputados open-data API by name. Returns "
            "matching federal deputies (party, UF, email, profile) — useful "
            "for PEP screening. Free, no key."
        ),
        "categories": ["Brazil", "Politics"],
        "group": [],
        "consumes": ["HUMAN_NAME"],
        "produces": ["HUMAN_NAME", "URL", "EMAILADDR", "RAW_RIR_DATA"],
    },
    {
        "id": "brazuca_querido_diario",
        "name": "Querido Diário — diários oficiais (live)",
        "description": (
            "Full-text search a name / company across thousands of Brazilian "
            "municipal official gazettes via the Querido Diário API (Open "
            "Knowledge Brasil). Free, no key."
        ),
        "categories": ["Brazil", "Search"],
        "group": [],
        "consumes": ["HUMAN_NAME", "COMPANY_NAME", "AFFILIATE_COMPANY_NAME"],
        "produces": ["URL", "RAW_RIR_DATA"],
    },
]


# ──────────────────────────────────────────────────────────────────────
#  Catalogue matching (brazuca_resources)
# ──────────────────────────────────────────────────────────────────────

def _matches(rule: str, canonical: str, label: str) -> bool:
    if rule == "*":
        return True
    if ":" in rule:
        etype, pat_name = rule.split(":", 1)
        if etype != canonical:
            return False
        pat = PATTERNS.get(pat_name)
        if pat is None:
            return False
        return pat.search(label) is not None
    return rule == canonical


def _find_matching(canonical: str, label: str) -> List[dict]:
    out = []
    for r in RESOURCES:
        if any(_matches(rule, canonical, label) for rule in r["applies_to"]):
            out.append(r)
    return out


def _run_resources(event_type: str, data: str) -> List[dict[str, Any]]:
    canonical = SF_TO_CANONICAL.get(event_type, event_type)
    matches = _find_matching(canonical, data)
    events: List[dict[str, Any]] = []
    for r in matches:
        events.append(
            {
                "event_type": "URL",
                # Human title rather than the raw URL — reads better on canvas.
                "data": r["title"],
                "module": "brazuca_resources",
                "confidence": 80,
                "source_event_type": event_type,
                "extra": {
                    "url": r["url"],
                    "title": r["title"],
                    "description": r["description"],
                    "category": r["category"],
                    "resource_id": r["id"],
                },
            }
        )
    return events


# ──────────────────────────────────────────────────────────────────────
#  HTTP helper (urllib, no extra deps)
# ──────────────────────────────────────────────────────────────────────

_DEFAULT_HEADERS = {
    "User-Agent": "maltrace-brazuca/0.2 (+https://github.com/) ",
    "Accept": "application/json,text/plain;q=0.9,*/*;q=0.5",
}
_HTTP_TIMEOUT = 12


def _http_get_json(url: str, *, headers: Optional[dict] = None) -> tuple[int, Any]:
    """GET url and return (status, parsed-json-or-text).

    Returns the body as a string when not JSON-decodable. Network and HTTP
    errors are raised; the caller decides how to translate them into events.
    """
    req = urllib.request.Request(url, headers={**_DEFAULT_HEADERS, **(headers or {})})
    try:
        with urllib.request.urlopen(req, timeout=_HTTP_TIMEOUT) as resp:
            raw = resp.read()
            status = resp.status
    except urllib.error.HTTPError as e:
        raw = e.read() if hasattr(e, "read") else b""
        status = e.code
    text = raw.decode("utf-8", errors="replace")
    try:
        return status, json.loads(text)
    except json.JSONDecodeError:
        return status, text


# ──────────────────────────────────────────────────────────────────────
#  Pattern extractors
# ──────────────────────────────────────────────────────────────────────

def _digits(s: str) -> str:
    return re.sub(r"\D+", "", s or "")


def _extract_cnpj(label: str) -> Optional[str]:
    d = _digits(label)
    if len(d) == 14:
        return d
    return None


def _extract_cep(label: str) -> Optional[str]:
    m = PATTERNS["cep"].search(label or "")
    if not m:
        return None
    return _digits(m.group(0)) or None


def _extract_ddd(phone: str) -> Optional[str]:
    """Pull the 2-digit DDD from a Brazilian phone number.

    Strips a leading 55 country code if present. Returns None if we can't
    confidently identify two leading area-code digits.
    """
    d = _digits(phone)
    if not d:
        return None
    if d.startswith("55") and len(d) in (12, 13):
        d = d[2:]
    if len(d) in (10, 11):
        return d[:2]
    if len(d) == 2:
        return d
    return None


# ──────────────────────────────────────────────────────────────────────
#  BrasilAPI scrapers
# ──────────────────────────────────────────────────────────────────────

_BRASILAPI = "https://brasilapi.com.br/api"


def _run_brasilapi_cnpj(event_type: str, data: str) -> List[dict[str, Any]]:
    cnpj = _extract_cnpj(data)
    if not cnpj:
        return [{
            "event_type": "RAW_RIR_DATA",
            "data": f"brasilapi_cnpj: no 14-digit CNPJ found in {data!r}",
            "module": "brazuca_brasilapi_cnpj",
            "confidence": 30,
            "source_event_type": event_type,
            "extra": {"input": data},
        }]

    status, body = _http_get_json(f"{_BRASILAPI}/cnpj/v1/{cnpj}")
    if status != 200 or not isinstance(body, dict):
        return [{
            "event_type": "RAW_RIR_DATA",
            "data": f"brasilapi_cnpj: status={status} for {cnpj}",
            "module": "brazuca_brasilapi_cnpj",
            "confidence": 30,
            "source_event_type": event_type,
            "extra": {"status": status, "body": body, "cnpj": cnpj},
        }]

    events: List[dict[str, Any]] = []
    razao = body.get("razao_social") or body.get("nome_fantasia")
    if razao:
        events.append({
            "event_type": "COMPANY_NAME",
            "data": razao,
            "module": "brazuca_brasilapi_cnpj",
            "confidence": 95,
            "source_event_type": event_type,
            "extra": {
                "cnpj": cnpj,
                "razao_social": body.get("razao_social"),
                "nome_fantasia": body.get("nome_fantasia"),
                "situacao_cadastral": body.get("descricao_situacao_cadastral"),
                "data_situacao": body.get("data_situacao_cadastral"),
                "cnae_principal": body.get("cnae_fiscal_descricao"),
                "porte": body.get("porte"),
            },
        })

    addr_parts = [
        body.get("descricao_tipo_de_logradouro"),
        body.get("logradouro"),
        body.get("numero"),
        body.get("complemento"),
        body.get("bairro"),
        body.get("municipio"),
        body.get("uf"),
        body.get("cep"),
    ]
    addr = ", ".join(p for p in addr_parts if p)
    if addr:
        events.append({
            "event_type": "PHYSICAL_ADDRESS",
            "data": addr,
            "module": "brazuca_brasilapi_cnpj",
            "confidence": 90,
            "source_event_type": event_type,
            "extra": {
                "cnpj": cnpj,
                "cep": body.get("cep"),
                "uf": body.get("uf"),
                "municipio": body.get("municipio"),
            },
        })

    for fld in ("ddd_telefone_1", "ddd_telefone_2", "ddd_fax"):
        phone = body.get(fld)
        if phone:
            events.append({
                "event_type": "PHONE_NUMBER",
                "data": phone,
                "module": "brazuca_brasilapi_cnpj",
                "confidence": 80,
                "source_event_type": event_type,
                "extra": {"cnpj": cnpj, "field": fld},
            })

    email = body.get("email")
    if email:
        events.append({
            "event_type": "EMAILADDR",
            "data": email,
            "module": "brazuca_brasilapi_cnpj",
            "confidence": 80,
            "source_event_type": event_type,
            "extra": {"cnpj": cnpj},
        })

    for q in body.get("qsa") or []:
        # `qsa` = quadro de sócios e administradores.
        nome = q.get("nome_socio") or q.get("nome")
        if nome:
            events.append({
                "event_type": "HUMAN_NAME",
                "data": nome,
                "module": "brazuca_brasilapi_cnpj",
                "confidence": 85,
                "source_event_type": event_type,
                "extra": {
                    "cnpj": cnpj,
                    "qualificacao": q.get("qualificacao_socio")
                    or q.get("codigo_qualificacao_socio"),
                    "faixa_etaria": q.get("faixa_etaria"),
                    "data_entrada_sociedade": q.get("data_entrada_sociedade"),
                },
            })

    events.append({
        "event_type": "RAW_RIR_DATA",
        "data": f"brasilapi_cnpj {cnpj}: {razao or '(no razao)'}",
        "module": "brazuca_brasilapi_cnpj",
        "confidence": 95,
        "source_event_type": event_type,
        "extra": {"cnpj": cnpj, "body": body},
    })
    return events


def _run_brasilapi_cep(event_type: str, data: str) -> List[dict[str, Any]]:
    cep = _extract_cep(data)
    if not cep:
        return [{
            "event_type": "RAW_RIR_DATA",
            "data": f"brasilapi_cep: no CEP pattern in {data!r}",
            "module": "brazuca_brasilapi_cep",
            "confidence": 30,
            "source_event_type": event_type,
            "extra": {"input": data},
        }]

    status, body = _http_get_json(f"{_BRASILAPI}/cep/v2/{cep}")
    if status != 200 or not isinstance(body, dict):
        return [{
            "event_type": "RAW_RIR_DATA",
            "data": f"brasilapi_cep: status={status} for {cep}",
            "module": "brazuca_brasilapi_cep",
            "confidence": 30,
            "source_event_type": event_type,
            "extra": {"status": status, "body": body, "cep": cep},
        }]

    addr_parts = [body.get("street"), body.get("neighborhood"),
                  body.get("city"), body.get("state"), body.get("cep")]
    addr = ", ".join(p for p in addr_parts if p)
    events: List[dict[str, Any]] = [{
        "event_type": "PHYSICAL_ADDRESS",
        "data": addr or cep,
        "module": "brazuca_brasilapi_cep",
        "confidence": 90,
        "source_event_type": event_type,
        "extra": {"cep": cep, **body},
    }]
    loc = body.get("location") or {}
    coords = loc.get("coordinates") or {}
    lat, lng = coords.get("latitude"), coords.get("longitude")
    if lat and lng:
        events.append({
            "event_type": "PHYSICAL_COORDINATES",
            "data": f"{lat},{lng}",
            "module": "brazuca_brasilapi_cep",
            "confidence": 85,
            "source_event_type": event_type,
            "extra": {"cep": cep, "lat": lat, "lng": lng},
        })
    if body.get("state"):
        events.append({
            "event_type": "GEOINFO",
            "data": f"BR / {body.get('state')} / {body.get('city')}",
            "module": "brazuca_brasilapi_cep",
            "confidence": 90,
            "source_event_type": event_type,
            "extra": {"cep": cep, "state": body.get("state"),
                      "city": body.get("city")},
        })
    events.append({
        "event_type": "COUNTRY_NAME",
        "data": "Brazil",
        "module": "brazuca_brasilapi_cep",
        "confidence": 99,
        "source_event_type": event_type,
        "extra": {"cep": cep},
    })
    return events


def _run_brasilapi_ddd(event_type: str, data: str) -> List[dict[str, Any]]:
    ddd = _extract_ddd(data)
    if not ddd:
        return [{
            "event_type": "RAW_RIR_DATA",
            "data": f"brasilapi_ddd: no DDD in {data!r}",
            "module": "brazuca_brasilapi_ddd",
            "confidence": 30,
            "source_event_type": event_type,
            "extra": {"input": data},
        }]

    status, body = _http_get_json(f"{_BRASILAPI}/ddd/v1/{ddd}")
    if status != 200 or not isinstance(body, dict):
        return [{
            "event_type": "RAW_RIR_DATA",
            "data": f"brasilapi_ddd: status={status} for {ddd}",
            "module": "brazuca_brasilapi_ddd",
            "confidence": 30,
            "source_event_type": event_type,
            "extra": {"status": status, "body": body, "ddd": ddd},
        }]

    state = body.get("state")
    cities = body.get("cities") or []
    events: List[dict[str, Any]] = [{
        "event_type": "GEOINFO",
        "data": f"DDD {ddd} → {state} ({len(cities)} cidades)",
        "module": "brazuca_brasilapi_ddd",
        "confidence": 90,
        "source_event_type": event_type,
        "extra": {"ddd": ddd, "state": state, "cities": cities},
    }, {
        "event_type": "COUNTRY_NAME",
        "data": "Brazil",
        "module": "brazuca_brasilapi_ddd",
        "confidence": 99,
        "source_event_type": event_type,
        "extra": {"ddd": ddd},
    }]
    return events


def _run_brasilapi_bank(event_type: str, data: str) -> List[dict[str, Any]]:
    d = _digits(data)
    if not (1 <= len(d) <= 3):
        return [{
            "event_type": "RAW_RIR_DATA",
            "data": f"brasilapi_bank: not a 3-digit bank code {data!r}",
            "module": "brazuca_brasilapi_bank",
            "confidence": 30,
            "source_event_type": event_type,
            "extra": {"input": data},
        }]
    status, body = _http_get_json(f"{_BRASILAPI}/banks/v1/{d}")
    if status != 200 or not isinstance(body, dict):
        return [{
            "event_type": "RAW_RIR_DATA",
            "data": f"brasilapi_bank: status={status} for {d}",
            "module": "brazuca_brasilapi_bank",
            "confidence": 30,
            "source_event_type": event_type,
            "extra": {"status": status, "body": body, "code": d},
        }]
    return [{
        "event_type": "COMPANY_NAME",
        "data": body.get("fullName") or body.get("name") or f"Banco {d}",
        "module": "brazuca_brasilapi_bank",
        "confidence": 95,
        "source_event_type": event_type,
        "extra": {"code": d, **body},
    }]


# ──────────────────────────────────────────────────────────────────────
#  ViaCEP
# ──────────────────────────────────────────────────────────────────────

def _run_viacep(event_type: str, data: str) -> List[dict[str, Any]]:
    cep = _extract_cep(data)
    if not cep:
        return [{
            "event_type": "RAW_RIR_DATA",
            "data": f"viacep: no CEP in {data!r}",
            "module": "brazuca_viacep",
            "confidence": 30,
            "source_event_type": event_type,
            "extra": {"input": data},
        }]
    status, body = _http_get_json(f"https://viacep.com.br/ws/{cep}/json/")
    if status != 200 or not isinstance(body, dict) or body.get("erro"):
        return [{
            "event_type": "RAW_RIR_DATA",
            "data": f"viacep: status={status} for {cep}",
            "module": "brazuca_viacep",
            "confidence": 30,
            "source_event_type": event_type,
            "extra": {"status": status, "body": body, "cep": cep},
        }]

    parts = [body.get("logradouro"), body.get("complemento"),
             body.get("bairro"), body.get("localidade"),
             body.get("uf"), body.get("cep")]
    addr = ", ".join(p for p in parts if p)
    return [{
        "event_type": "PHYSICAL_ADDRESS",
        "data": addr or cep,
        "module": "brazuca_viacep",
        "confidence": 90,
        "source_event_type": event_type,
        "extra": {"cep": cep, **body},
    }, {
        "event_type": "GEOINFO",
        "data": f"BR / {body.get('uf')} / {body.get('localidade')}",
        "module": "brazuca_viacep",
        "confidence": 90,
        "source_event_type": event_type,
        "extra": {"cep": cep, **body},
    }, {
        "event_type": "COUNTRY_NAME",
        "data": "Brazil",
        "module": "brazuca_viacep",
        "confidence": 99,
        "source_event_type": event_type,
        "extra": {"cep": cep},
    }]


# ──────────────────────────────────────────────────────────────────────
#  Hudson Rock — Cavalier (free OSINT endpoints)
#
#  Documented at https://cavalier.hudsonrock.com/api/json/v2/osint-tools .
#  Returns JSON with infection counts and optionally first/last-seen.
# ──────────────────────────────────────────────────────────────────────

_HR_BASE = "https://cavalier.hudsonrock.com/api/json/v2/osint-tools"


def _run_hudson_rock(event_type: str, data: str, *, kind: str) -> List[dict[str, Any]]:
    """`kind` is one of 'email', 'domain', 'username'."""
    selector = data.strip()
    if not selector:
        return []
    if kind == "email":
        url = f"{_HR_BASE}/search-by-email?email={urllib.parse.quote(selector)}"
    elif kind == "domain":
        url = f"{_HR_BASE}/search-by-domain?domain={urllib.parse.quote(selector)}"
    elif kind == "username":
        url = f"{_HR_BASE}/search-by-username?username={urllib.parse.quote(selector)}"
    else:
        raise ValueError(f"unknown hudson-rock kind: {kind}")

    status, body = _http_get_json(url)
    module_id = f"brazuca_hudson_rock_{kind}"
    if status != 200 or not isinstance(body, dict):
        return [{
            "event_type": "RAW_RIR_DATA",
            "data": f"hudson_rock_{kind}: status={status} for {selector}",
            "module": module_id,
            "confidence": 30,
            "source_event_type": event_type,
            "extra": {"status": status, "body": body, "selector": selector},
        }]

    # Common shape: { "stealers": [...], "total": N, "message": "..."} for email/username.
    # Domain returns more aggregate stats.
    events: List[dict[str, Any]] = []
    total = (
        body.get("total")
        or body.get("totalStealers")
        or len(body.get("stealers") or [])
        or len(body.get("employees") or [])
        or len(body.get("users") or [])
        or 0
    )
    summary = f"hudson_rock_{kind} {selector}: total={total}"
    events.append({
        "event_type": "RAW_RIR_DATA",
        "data": summary,
        "module": module_id,
        "confidence": 90 if total else 60,
        "source_event_type": event_type,
        "extra": {"selector": selector, "body": body},
    })

    if kind == "email" and total:
        events.append({
            "event_type": "EMAILADDR_COMPROMISED",
            "data": f"{selector} [Hudson Rock infostealer]",
            "module": module_id,
            "confidence": 95,
            "source_event_type": event_type,
            "extra": {"selector": selector, "total": total, "body": body},
        })
    if kind == "domain":
        # Domain endpoint reports per-employee / per-user counts; surface
        # leaked employee credentials when present.
        for u in (body.get("employees") or [])[:50]:
            login = u.get("login") if isinstance(u, dict) else None
            if login and "@" in login:
                events.append({
                    "event_type": "EMAILADDR_COMPROMISED",
                    "data": login,
                    "module": module_id,
                    "confidence": 85,
                    "source_event_type": event_type,
                    "extra": {"selector": selector, "entry": u},
                })

    return events


# ──────────────────────────────────────────────────────────────────────
#  CPF / CNPJ offline validation
#
#  Pure check-digit arithmetic (mod 11) — no network. The CPF's 9th digit
#  encodes the Receita Federal fiscal region, which maps to a set of states.
# ──────────────────────────────────────────────────────────────────────

# 9th CPF digit → fiscal region → states of issue.
_CPF_REGIONS = {
    "0": "RS",
    "1": "DF, GO, MS, MT, TO",
    "2": "AC, AM, AP, PA, RO, RR",
    "3": "CE, MA, PI",
    "4": "AL, PB, PE, RN",
    "5": "BA, SE",
    "6": "MG",
    "7": "ES, RJ",
    "8": "SP",
    "9": "PR, SC",
}


def _cpf_check_digits(base9: str) -> str:
    """Return the two CPF check digits for the first 9 digits."""
    def dv(nums: str, start_weight: int) -> str:
        total = sum(int(n) * w for n, w in
                    zip(nums, range(start_weight, 1, -1)))
        rem = (total * 10) % 11
        return "0" if rem == 10 else str(rem)
    d1 = dv(base9, 10)
    d2 = dv(base9 + d1, 11)
    return d1 + d2


def _extract_cpf(label: str) -> Optional[str]:
    d = _digits(label)
    return d if len(d) == 11 else None


def _run_cpf_validate(event_type: str, data: str) -> List[dict[str, Any]]:
    cpf = _extract_cpf(data)
    if not cpf:
        return [{
            "event_type": "RAW_RIR_DATA",
            "data": f"cpf_validate: no 11-digit CPF found in {data!r}",
            "module": "brazuca_cpf_validate",
            "confidence": 30,
            "source_event_type": event_type,
            "extra": {"input": data},
        }]

    # All-equal digits (000…, 111…) pass the math but are never valid.
    repeated = len(set(cpf)) == 1
    valid = (not repeated) and _cpf_check_digits(cpf[:9]) == cpf[9:]
    region_digit = cpf[8]
    states = _CPF_REGIONS.get(region_digit)
    formatted = f"{cpf[:3]}.{cpf[3:6]}.{cpf[6:9]}-{cpf[9:]}"

    events: List[dict[str, Any]] = [{
        "event_type": "RAW_RIR_DATA",
        "data": (f"CPF {formatted}: {'válido' if valid else 'INVÁLIDO'}"
                 f"{f' · região fiscal {region_digit} ({states})' if states else ''}"),
        "module": "brazuca_cpf_validate",
        "confidence": 95 if valid else 70,
        "source_event_type": event_type,
        "extra": {"cpf": cpf, "formatted": formatted, "valid": valid,
                  "fiscal_region": region_digit, "fiscal_region_states": states},
    }]
    if valid and states:
        events.append({
            "event_type": "GEOINFO",
            "data": f"BR / região fiscal {region_digit}: {states}",
            "module": "brazuca_cpf_validate",
            "confidence": 60,
            "source_event_type": event_type,
            "extra": {"cpf": cpf, "fiscal_region": region_digit,
                      "states": states, "note": "região fiscal de emissão do CPF"},
        })
    return events


def _cnpj_check_digits(base12: str) -> str:
    """Return the two CNPJ check digits for the first 12 digits."""
    def dv(nums: str) -> str:
        # 13-weight base so the second-digit pass (13 digits) slices the full
        # sequence; the first-digit pass (12 digits) takes the trailing 12.
        weights = [6, 5, 4, 3, 2, 9, 8, 7, 6, 5, 4, 3, 2][-len(nums):]
        total = sum(int(n) * w for n, w in zip(nums, weights))
        rem = total % 11
        return "0" if rem < 2 else str(11 - rem)
    d1 = dv(base12)
    d2 = dv(base12 + d1)
    return d1 + d2


def _run_cnpj_validate(event_type: str, data: str) -> List[dict[str, Any]]:
    cnpj = _extract_cnpj(data)
    if not cnpj:
        return [{
            "event_type": "RAW_RIR_DATA",
            "data": f"cnpj_validate: no 14-digit CNPJ found in {data!r}",
            "module": "brazuca_cnpj_validate",
            "confidence": 30,
            "source_event_type": event_type,
            "extra": {"input": data},
        }]

    repeated = len(set(cnpj)) == 1
    valid = (not repeated) and _cnpj_check_digits(cnpj[:12]) == cnpj[12:]
    branch = cnpj[8:12]
    is_matriz = branch == "0001"
    formatted = f"{cnpj[:2]}.{cnpj[2:5]}.{cnpj[5:8]}/{cnpj[8:12]}-{cnpj[12:]}"
    kind = "matriz" if is_matriz else f"filial nº {int(branch)}"

    return [{
        "event_type": "RAW_RIR_DATA",
        "data": (f"CNPJ {formatted}: {'válido' if valid else 'INVÁLIDO'} · {kind}"),
        "module": "brazuca_cnpj_validate",
        "confidence": 95 if valid else 70,
        "source_event_type": event_type,
        "extra": {"cnpj": cnpj, "formatted": formatted, "valid": valid,
                  "branch": branch, "is_matriz": is_matriz, "kind": kind},
    }]


# ──────────────────────────────────────────────────────────────────────
#  Registro.br domain status (via BrasilAPI, no key)
# ──────────────────────────────────────────────────────────────────────

def _run_registrobr(event_type: str, data: str) -> List[dict[str, Any]]:
    domain = (data or "").strip().lower()
    if "://" in domain:
        domain = domain.split("://", 1)[1]
    domain = domain.split("/", 1)[0].split("@")[-1].rstrip(".")
    if not domain.endswith(".br"):
        return [{
            "event_type": "RAW_RIR_DATA",
            "data": f"registrobr: {data!r} is not a .br domain",
            "module": "brazuca_registrobr",
            "confidence": 30,
            "source_event_type": event_type,
            "extra": {"input": data},
        }]

    status, body = _http_get_json(f"{_BRASILAPI}/registrobr/v1/{urllib.parse.quote(domain)}")
    if status != 200 or not isinstance(body, dict):
        return [{
            "event_type": "RAW_RIR_DATA",
            "data": f"registrobr: status={status} for {domain}",
            "module": "brazuca_registrobr",
            "confidence": 30,
            "source_event_type": event_type,
            "extra": {"status": status, "body": body, "domain": domain},
        }]

    reg_status = body.get("status") or body.get("status_code")
    expires = body.get("expires-at") or body.get("expiresAt")
    hosts = body.get("hosts") or []
    events: List[dict[str, Any]] = [{
        "event_type": "RAW_RIR_DATA",
        "data": f"registro.br {domain}: status={reg_status}"
                f"{f', expira {expires}' if expires else ''}",
        "module": "brazuca_registrobr",
        "confidence": 90,
        "source_event_type": event_type,
        "extra": {"domain": domain, "body": body},
    }]
    if expires:
        events.append({
            "event_type": "DOMAIN_REGISTRAR",
            "data": f"Registro.br — {domain} (expira {expires})",
            "module": "brazuca_registrobr",
            "confidence": 85,
            "source_event_type": event_type,
            "extra": {"domain": domain, "registry": "Registro.br",
                      "expires_at": expires, "status": reg_status},
        })
    for ns in hosts:
        if isinstance(ns, str) and ns:
            events.append({
                "event_type": "PROVIDER_DNS",
                "data": ns,
                "module": "brazuca_registrobr",
                "confidence": 80,
                "source_event_type": event_type,
                "extra": {"domain": domain, "record": "NS"},
            })
    return events


# ──────────────────────────────────────────────────────────────────────
#  Câmara dos Deputados — search by name (open data, no key)
# ──────────────────────────────────────────────────────────────────────

def _run_camara_deputado(event_type: str, data: str) -> List[dict[str, Any]]:
    nome = (data or "").strip()
    if len(nome) < 3:
        return [{
            "event_type": "RAW_RIR_DATA",
            "data": f"camara_deputado: name too short {data!r}",
            "module": "brazuca_camara_deputado",
            "confidence": 30,
            "source_event_type": event_type,
            "extra": {"input": data},
        }]

    url = ("https://dadosabertos.camara.leg.br/api/v2/deputados?"
           + urllib.parse.urlencode({"nome": nome, "ordem": "ASC",
                                     "ordenarPor": "nome", "itens": "20"}))
    status, body = _http_get_json(url, headers={"Accept": "application/json"})
    deputados = body.get("dados") if isinstance(body, dict) else None
    if status != 200 or not isinstance(deputados, list):
        return [{
            "event_type": "RAW_RIR_DATA",
            "data": f"camara_deputado: status={status} for {nome!r}",
            "module": "brazuca_camara_deputado",
            "confidence": 30,
            "source_event_type": event_type,
            "extra": {"status": status, "body": body, "nome": nome},
        }]

    events: List[dict[str, Any]] = []
    for dep in deputados:
        nm = dep.get("nome")
        if not nm:
            continue
        party = dep.get("siglaPartido")
        uf = dep.get("siglaUf")
        events.append({
            "event_type": "HUMAN_NAME",
            "data": f"{nm} ({party}-{uf})" if party else nm,
            "module": "brazuca_camara_deputado",
            "confidence": 80,
            "source_event_type": event_type,
            "extra": {"deputado_id": dep.get("id"), "nome": nm,
                      "partido": party, "uf": uf, "url": dep.get("uri"),
                      "is_pep": True, "cargo": "Deputado Federal"},
        })
        if dep.get("uri"):
            events.append({
                "event_type": "URL",
                "data": dep["uri"],
                "module": "brazuca_camara_deputado",
                "confidence": 75,
                "source_event_type": event_type,
                "extra": {"url": dep["uri"], "title": f"Câmara — {nm}",
                          "description": f"Perfil de {nm} ({party}-{uf})"},
            })
        if dep.get("email"):
            events.append({
                "event_type": "EMAILADDR",
                "data": dep["email"],
                "module": "brazuca_camara_deputado",
                "confidence": 75,
                "source_event_type": event_type,
                "extra": {"deputado": nm, "source": "camara.leg.br"},
            })

    events.append({
        "event_type": "RAW_RIR_DATA",
        "data": f"camara_deputado {nome!r}: {len(deputados)} deputado(s) encontrado(s)",
        "module": "brazuca_camara_deputado",
        "confidence": 85 if deputados else 50,
        "source_event_type": event_type,
        "extra": {"nome": nome, "count": len(deputados)},
    })
    return events


# ──────────────────────────────────────────────────────────────────────
#  Querido Diário — municipal official-gazette full-text search (no key)
# ──────────────────────────────────────────────────────────────────────

def _run_querido_diario(event_type: str, data: str) -> List[dict[str, Any]]:
    term = (data or "").strip()
    if len(term) < 3:
        return [{
            "event_type": "RAW_RIR_DATA",
            "data": f"querido_diario: query too short {data!r}",
            "module": "brazuca_querido_diario",
            "confidence": 30,
            "source_event_type": event_type,
            "extra": {"input": data},
        }]

    # Quote multi-word names so the API treats them as a phrase.
    q = f'"{term}"' if " " in term else term
    url = ("https://queridodiario.ok.org.br/api/gazettes?"
           + urllib.parse.urlencode({"querystring": q, "size": "10",
                                     "sort_by": "descending_date"}))
    # Send a browser-like UA + site Origin/Referer: the public API sits behind
    # a WAF that 403s bare programmatic requests from some networks.
    status, body = _http_get_json(url, headers={
        "Accept": "application/json",
        "User-Agent": ("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 "
                       "(KHTML, like Gecko) Chrome/124.0 Safari/537.36"),
        "Origin": "https://queridodiario.ok.org.br",
        "Referer": "https://queridodiario.ok.org.br/",
    })
    if status != 200 or not isinstance(body, dict):
        return [{
            "event_type": "RAW_RIR_DATA",
            "data": f"querido_diario: status={status} for {term!r}",
            "module": "brazuca_querido_diario",
            "confidence": 30,
            "source_event_type": event_type,
            "extra": {"status": status, "body": body, "term": term},
        }]

    gazettes = body.get("gazettes") or []
    total = body.get("total_gazettes")
    if total is None:
        total = len(gazettes)

    events: List[dict[str, Any]] = []
    for g in gazettes:
        gurl = g.get("url") or g.get("txt_url")
        territory = g.get("territory_name")
        state = g.get("state_code")
        gdate = g.get("date")
        if not gurl:
            continue
        excerpts = g.get("excerpts") or []
        excerpt = (excerpts[0] if excerpts else "")[:200]
        events.append({
            "event_type": "URL",
            "data": gurl,
            "module": "brazuca_querido_diario",
            "confidence": 75,
            "source_event_type": event_type,
            "extra": {"url": gurl, "title": f"Diário Oficial — {territory}/{state} ({gdate})",
                      "description": excerpt, "territory": territory,
                      "state": state, "date": gdate, "term": term},
        })

    events.append({
        "event_type": "RAW_RIR_DATA",
        "data": f"querido_diario {term!r}: {total} diário(s) com menções",
        "module": "brazuca_querido_diario",
        "confidence": 85 if gazettes else 50,
        "source_event_type": event_type,
        "extra": {"term": term, "total_gazettes": total,
                  "shown": len(events)},
    })
    return events


# ──────────────────────────────────────────────────────────────────────
#  Dispatch
# ──────────────────────────────────────────────────────────────────────

def _dispatch_run(module: str, event_type: str, data: str) -> List[dict[str, Any]]:
    if module == "brazuca_resources":
        return _run_resources(event_type, data)
    if module == "brazuca_brasilapi_cnpj":
        return _run_brasilapi_cnpj(event_type, data)
    if module == "brazuca_brasilapi_cep":
        return _run_brasilapi_cep(event_type, data)
    if module == "brazuca_brasilapi_ddd":
        return _run_brasilapi_ddd(event_type, data)
    if module == "brazuca_brasilapi_bank":
        return _run_brasilapi_bank(event_type, data)
    if module == "brazuca_viacep":
        return _run_viacep(event_type, data)
    if module == "brazuca_hudson_rock_email":
        return _run_hudson_rock(event_type, data, kind="email")
    if module == "brazuca_hudson_rock_domain":
        return _run_hudson_rock(event_type, data, kind="domain")
    if module == "brazuca_hudson_rock_username":
        return _run_hudson_rock(event_type, data, kind="username")
    if module == "brazuca_cpf_validate":
        return _run_cpf_validate(event_type, data)
    if module == "brazuca_cnpj_validate":
        return _run_cnpj_validate(event_type, data)
    if module == "brazuca_registrobr":
        return _run_registrobr(event_type, data)
    if module == "brazuca_camara_deputado":
        return _run_camara_deputado(event_type, data)
    if module == "brazuca_querido_diario":
        return _run_querido_diario(event_type, data)
    raise ValueError(f"unknown module: {module}")


# ──────────────────────────────────────────────────────────────────────
#  Subcommand entry points
# ──────────────────────────────────────────────────────────────────────

def cmd_list_transforms(_: argparse.Namespace) -> int:
    json.dump({"transforms": TRANSFORMS, "skipped": []}, sys.stdout)
    sys.stdout.write("\n")
    return 0


def cmd_run(args: argparse.Namespace) -> int:
    try:
        events = _dispatch_run(args.module, args.event_type, args.data)
    except Exception as exc:  # noqa: BLE001
        json.dump(
            {
                "ok": False,
                "error": f"{type(exc).__name__}: {exc}",
                "traceback": traceback.format_exc(),
                "events": [],
            },
            sys.stdout,
        )
        sys.stdout.write("\n")
        return 3
    json.dump({"ok": True, "events": events}, sys.stdout)
    sys.stdout.write("\n")
    return 0


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(prog="maltrace-brazuca-adapter")
    sub = parser.add_subparsers(dest="cmd", required=True)
    sub.add_parser("list-transforms", help="List brazuca transforms.")
    run = sub.add_parser("run", help="Run a brazuca transform on one entity.")
    run.add_argument("--module", required=True)
    run.add_argument("--event-type", required=True)
    run.add_argument("--data", required=True)
    run.add_argument("--opts", default=None)

    args = parser.parse_args(argv)
    if args.cmd == "list-transforms":
        return cmd_list_transforms(args)
    if args.cmd == "run":
        return cmd_run(args)
    parser.print_help(sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())
