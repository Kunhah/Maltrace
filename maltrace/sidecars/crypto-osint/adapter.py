"""
maltrace ↔ crypto-osint adapter.

Live OSINT on a Bitcoin or Ethereum address using free public block
explorers (no API keys required):

  * mempool.space    — BTC address stats: tx count, balance, first/last seen.
  * ethplorer        — ETH address stats via the public `freekey` token.
  * Plus a small curated "abuse / labels" link set surfaced as URL entities
    (Chainabuse, Etherscan label cloud, Bitcoinabuse) so an analyst can
    pivot into reputation databases with one click.

Auto-detects address type from the input label:

  * BTC legacy / SegWit: matches `^[13][...]{25,34}$` or `^bc1[...]`.
  * ETH:                 matches `^0x[a-fA-F0-9]{40}$`.

Same one-shot JSON protocol as the other sidecars.
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
#  Address-format detection
# ──────────────────────────────────────────────────────────────────────

_BTC_LEGACY = re.compile(r"^[13][a-km-zA-HJ-NP-Z1-9]{25,34}$")
_BTC_BECH32 = re.compile(r"^(?:bc1|tb1)[ac-hj-np-z02-9]{6,87}$")
_ETH = re.compile(r"^0x[a-fA-F0-9]{40}$")


def _detect_chain(addr: str) -> Optional[str]:
    s = (addr or "").strip()
    if _ETH.match(s):
        return "eth"
    if _BTC_LEGACY.match(s) or _BTC_BECH32.match(s):
        return "btc"
    return None


# ──────────────────────────────────────────────────────────────────────
#  HTTP helper
# ──────────────────────────────────────────────────────────────────────

_HEADERS = {
    "User-Agent": "maltrace-crypto-osint/0.1 (+https://github.com/) ",
    "Accept": "application/json",
}
_TIMEOUT = 12


def _get_json(url: str) -> tuple[int, Any]:
    req = urllib.request.Request(url, headers=_HEADERS)
    try:
        with urllib.request.urlopen(req, timeout=_TIMEOUT) as resp:
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
#  Curated "open in browser" abuse / label resources
# ──────────────────────────────────────────────────────────────────────

def _abuse_urls(chain: str, addr: str) -> List[dict[str, str]]:
    if chain == "btc":
        return [
            {
                "title": f"mempool.space — {addr}",
                "url": f"https://mempool.space/address/{addr}",
                "description": "Block explorer (mempool.space) — txs, balance, UTXO set.",
            },
            {
                "title": f"blockchain.com — {addr}",
                "url": f"https://www.blockchain.com/btc/address/{addr}",
                "description": "Blockchain.com explorer.",
            },
            {
                "title": f"Chainabuse — {addr}",
                "url": f"https://www.chainabuse.com/address/{addr}",
                "description": "Community-sourced abuse-report database.",
            },
            {
                "title": f"BitcoinAbuse — {addr}",
                "url": f"https://www.bitcoinabuse.com/reports/{addr}",
                "description": "BitcoinAbuse historical reports for this address.",
            },
            {
                "title": f"Oxt.me — {addr}",
                "url": f"https://oxt.me/address/{addr}",
                "description": "OXT graph explorer (cluster heuristics).",
            },
        ]
    if chain == "eth":
        return [
            {
                "title": f"Etherscan — {addr}",
                "url": f"https://etherscan.io/address/{addr}",
                "description": "Etherscan address page (tx list, internal txs, labels).",
            },
            {
                "title": f"Ethplorer — {addr}",
                "url": f"https://ethplorer.io/address/{addr}",
                "description": "Ethplorer — ERC-20 holdings, NFT positions.",
            },
            {
                "title": f"Chainabuse — {addr}",
                "url": f"https://www.chainabuse.com/address/{addr}",
                "description": "Community-sourced abuse-report database.",
            },
            {
                "title": f"Breadcrumbs Graph — {addr}",
                "url": f"https://www.breadcrumbs.app/reports?address={addr}",
                "description": "Open-graph explorer for ETH wallets.",
            },
        ]
    return []


# ──────────────────────────────────────────────────────────────────────
#  Bitcoin: mempool.space
#
#  Endpoint: GET https://mempool.space/api/address/<addr>
#  Returns: {chain_stats: {funded_txo_sum, spent_txo_sum, tx_count, ...},
#            mempool_stats: {...}}
#  Amounts are in satoshis (1 BTC = 100_000_000 sat).
# ──────────────────────────────────────────────────────────────────────

def _btc_balance_btc(chain_stats: dict, mempool_stats: dict) -> float:
    funded = (chain_stats.get("funded_txo_sum") or 0) + (mempool_stats.get("funded_txo_sum") or 0)
    spent = (chain_stats.get("spent_txo_sum") or 0) + (mempool_stats.get("spent_txo_sum") or 0)
    return (funded - spent) / 100_000_000


def _run_btc_lookup(event_type: str, addr: str) -> List[dict[str, Any]]:
    status, body = _get_json(f"https://mempool.space/api/address/{urllib.parse.quote(addr)}")
    if status != 200 or not isinstance(body, dict):
        return [{
            "event_type": "RAW_RIR_DATA",
            "data": f"mempool.space: status={status} for {addr}",
            "module": "crypto_btc_lookup",
            "confidence": 30,
            "source_event_type": event_type,
            "extra": {"status": status, "body": body, "address": addr},
        }]

    chain_stats = body.get("chain_stats") or {}
    mempool_stats = body.get("mempool_stats") or {}
    bal_btc = _btc_balance_btc(chain_stats, mempool_stats)
    tx_count = (chain_stats.get("tx_count") or 0) + (mempool_stats.get("tx_count") or 0)

    events: List[dict[str, Any]] = []
    events.append({
        "event_type": "BITCOIN_BALANCE",
        "data": f"{bal_btc:.8f} BTC ({tx_count} tx)",
        "module": "crypto_btc_lookup",
        "confidence": 95,
        "source_event_type": event_type,
        "extra": {
            "address": addr,
            "balance_btc": bal_btc,
            "balance_sat": int(round(bal_btc * 100_000_000)),
            "tx_count": tx_count,
            "chain_stats": chain_stats,
            "mempool_stats": mempool_stats,
        },
    })
    events.append({
        "event_type": "RAW_RIR_DATA",
        "data": f"mempool.space {addr}: {bal_btc:.8f} BTC, {tx_count} tx",
        "module": "crypto_btc_lookup",
        "confidence": 90,
        "source_event_type": event_type,
        "extra": {"address": addr, "body": body},
    })
    for link in _abuse_urls("btc", addr):
        events.append({
            "event_type": "URL",
            "data": link["title"],
            "module": "crypto_btc_lookup",
            "confidence": 80,
            "source_event_type": event_type,
            "extra": link,
        })
    return events


# ──────────────────────────────────────────────────────────────────────
#  Ethereum: Ethplorer (public freekey) + Etherscan free txlist
#
#  Ethplorer `freekey` is rate-limited but no signup. Endpoint:
#    GET https://api.ethplorer.io/getAddressInfo/<addr>?apiKey=freekey
#  Etherscan `api?module=account&action=txlist&address=<a>&apikey=` works
#  rate-limited without a key for `txlist`/`balance` actions in practice,
#  but we stick to Ethplorer to stay deterministic.
# ──────────────────────────────────────────────────────────────────────

def _run_eth_lookup(event_type: str, addr: str) -> List[dict[str, Any]]:
    url = f"https://api.ethplorer.io/getAddressInfo/{urllib.parse.quote(addr)}?apiKey=freekey"
    status, body = _get_json(url)
    if status != 200 or not isinstance(body, dict):
        return [{
            "event_type": "RAW_RIR_DATA",
            "data": f"ethplorer: status={status} for {addr}",
            "module": "crypto_eth_lookup",
            "confidence": 30,
            "source_event_type": event_type,
            "extra": {"status": status, "body": body, "address": addr},
        }]
    if body.get("error"):
        return [{
            "event_type": "RAW_RIR_DATA",
            "data": f"ethplorer error: {body['error']} for {addr}",
            "module": "crypto_eth_lookup",
            "confidence": 30,
            "source_event_type": event_type,
            "extra": {"address": addr, "body": body},
        }]

    eth = (body.get("ETH") or {})
    bal_eth = float(eth.get("balance") or 0)
    tx_count = int(eth.get("totalIn") or 0) + int(eth.get("totalOut") or 0)
    tokens = body.get("tokens") or []

    events: List[dict[str, Any]] = []
    events.append({
        "event_type": "ETHEREUM_BALANCE",
        "data": f"{bal_eth:.8f} ETH ({tx_count} tx, {len(tokens)} ERC-20 holdings)",
        "module": "crypto_eth_lookup",
        "confidence": 95,
        "source_event_type": event_type,
        "extra": {
            "address": addr,
            "balance_eth": bal_eth,
            "tx_count": tx_count,
            "token_count": len(tokens),
            "tokens": [
                {
                    "symbol": (t.get("tokenInfo") or {}).get("symbol"),
                    "name": (t.get("tokenInfo") or {}).get("name"),
                    "balance": t.get("balance"),
                    "contract": (t.get("tokenInfo") or {}).get("address"),
                }
                for t in tokens[:50]
            ],
        },
    })
    events.append({
        "event_type": "RAW_RIR_DATA",
        "data": f"ethplorer {addr}: {bal_eth:.8f} ETH, {tx_count} tx",
        "module": "crypto_eth_lookup",
        "confidence": 90,
        "source_event_type": event_type,
        "extra": {"address": addr, "body": body},
    })
    for link in _abuse_urls("eth", addr):
        events.append({
            "event_type": "URL",
            "data": link["title"],
            "module": "crypto_eth_lookup",
            "confidence": 80,
            "source_event_type": event_type,
            "extra": link,
        })
    return events


# ──────────────────────────────────────────────────────────────────────
#  Address detect + classify (no live lookup)
# ──────────────────────────────────────────────────────────────────────

def _run_classify(event_type: str, addr: str) -> List[dict[str, Any]]:
    chain = _detect_chain(addr)
    if chain is None:
        return [{
            "event_type": "RAW_RIR_DATA",
            "data": f"crypto_classify: unknown address format {addr!r}",
            "module": "crypto_classify",
            "confidence": 30,
            "source_event_type": event_type,
            "extra": {"input": addr},
        }]
    typed = "BITCOIN_ADDRESS" if chain == "btc" else "ETHEREUM_ADDRESS"
    return [{
        "event_type": typed,
        "data": addr,
        "module": "crypto_classify",
        "confidence": 99,
        "source_event_type": event_type,
        "extra": {"chain": chain, "address": addr},
    }]


# ──────────────────────────────────────────────────────────────────────
#  "auto" dispatcher — detect chain and route to the right lookup.
# ──────────────────────────────────────────────────────────────────────

def _run_auto(event_type: str, addr: str) -> List[dict[str, Any]]:
    chain = _detect_chain(addr)
    if chain == "btc":
        return _run_btc_lookup(event_type, addr)
    if chain == "eth":
        return _run_eth_lookup(event_type, addr)
    return [{
        "event_type": "RAW_RIR_DATA",
        "data": f"crypto_auto_lookup: no supported chain for {addr!r}",
        "module": "crypto_auto_lookup",
        "confidence": 30,
        "source_event_type": event_type,
        "extra": {"input": addr},
    }]


# ──────────────────────────────────────────────────────────────────────
#  Transform manifest
# ──────────────────────────────────────────────────────────────────────

TRANSFORMS = [
    {
        "id": "crypto_auto_lookup",
        "name": "Crypto wallet lookup (auto BTC/ETH)",
        "description": (
            "Detect whether the input is a BTC or ETH address and run the "
            "appropriate explorer (mempool.space or ethplorer). Returns "
            "balance, tx count, ERC-20 holdings (ETH) and curated abuse / "
            "reputation links."
        ),
        "categories": ["Crypto", "Lookup"],
        "group": [],
        "consumes": [
            "BITCOIN_ADDRESS",
            "ETHEREUM_ADDRESS",
            "MALICIOUS_BITCOIN_ADDRESS",
            "ACCOUNT_EXTERNAL_OWNED",
        ],
        "produces": [
            "BITCOIN_BALANCE",
            "ETHEREUM_BALANCE",
            "URL",
            "RAW_RIR_DATA",
        ],
    },
    {
        "id": "crypto_btc_lookup",
        "name": "Bitcoin wallet lookup (mempool.space)",
        "description": (
            "Bitcoin-only variant — query mempool.space for balance, tx "
            "count and reputation pivots. No API key."
        ),
        "categories": ["Crypto", "Lookup"],
        "group": [],
        "consumes": ["BITCOIN_ADDRESS", "MALICIOUS_BITCOIN_ADDRESS"],
        "produces": ["BITCOIN_BALANCE", "URL", "RAW_RIR_DATA"],
    },
    {
        "id": "crypto_eth_lookup",
        "name": "Ethereum wallet lookup (ethplorer)",
        "description": (
            "Ethereum-only variant — query ethplorer's public freekey "
            "endpoint for ETH balance, tx count and ERC-20 holdings."
        ),
        "categories": ["Crypto", "Lookup"],
        "group": [],
        "consumes": ["ETHEREUM_ADDRESS"],
        "produces": ["ETHEREUM_BALANCE", "URL", "RAW_RIR_DATA"],
    },
    {
        "id": "crypto_classify",
        "name": "Crypto address classify (no network)",
        "description": (
            "Detect whether the input matches a Bitcoin (legacy/bech32) "
            "or Ethereum address format and re-emit as the typed entity. "
            "Useful to lift opaque labels into the crypto ontology."
        ),
        "categories": ["Crypto", "Detect"],
        "group": [],
        "consumes": ["ACCOUNT_EXTERNAL_OWNED", "RAW_RIR_DATA", "USERNAME"],
        "produces": ["BITCOIN_ADDRESS", "ETHEREUM_ADDRESS"],
    },
]


# ──────────────────────────────────────────────────────────────────────
#  Dispatch
# ──────────────────────────────────────────────────────────────────────

def _dispatch_run(module: str, event_type: str, data: str) -> List[dict]:
    if module == "crypto_auto_lookup":
        return _run_auto(event_type, data)
    if module == "crypto_btc_lookup":
        return _run_btc_lookup(event_type, data)
    if module == "crypto_eth_lookup":
        return _run_eth_lookup(event_type, data)
    if module == "crypto_classify":
        return _run_classify(event_type, data)
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
        json.dump({
            "ok": False,
            "error": f"{type(exc).__name__}: {exc}",
            "traceback": traceback.format_exc(),
            "events": [],
        }, sys.stdout)
        sys.stdout.write("\n")
        return 3
    json.dump({"ok": True, "events": events}, sys.stdout)
    sys.stdout.write("\n")
    return 0


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(prog="maltrace-crypto-osint-adapter")
    sub = parser.add_subparsers(dest="cmd", required=True)
    sub.add_parser("list-transforms", help="List crypto-osint transforms.")
    run = sub.add_parser("run", help="Run a crypto-osint transform on one address.")
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
