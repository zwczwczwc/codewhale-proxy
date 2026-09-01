#!/usr/bin/env python3
"""Safe, deterministic Responses cache/continuation A/B dry-run scaffold.

This file intentionally does not implement real traffic. ``--run-real`` is an
explicit NOT_IMPLEMENTED blocker, so an incomplete A-only experiment cannot be
mistaken for a complete A/B result. The default dry-run is deterministic and
never reads credentials or performs network/process I/O.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import sys
from typing import Any

A_URL = "http://clawbot:11434/v1/responses"
B_URL = "http://127.0.0.1:11449/v1/messages"
ALLOWED_UPSTREAM_PORTS = frozenset({11434})
ALLOWED_LOCAL_PORTS = frozenset({11449})
FORBIDDEN_LOCAL_PORTS = frozenset({11434, 11441})
FORBIDDEN_URLS = frozenset({
    "http://127.0.0.1:11434",
    "http://127.0.0.1:11441",
})
REAL_STATUS = "NOT_IMPLEMENTED"
DRY_RUN_MODEL = "gpt-5.6-luna"
MAX_BODY = 8 * 1024 * 1024


def digest(value: bytes | str) -> str:
    if isinstance(value, str):
        value = value.encode()
    return hashlib.sha256(value).hexdigest()[:16]


def safe_json(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def request_body() -> dict[str, Any]:
    """Return the fixed synthetic request used only for deterministic review."""
    fixed_input = "cache-ab synthetic segment " * 900
    return {
        "model": DRY_RUN_MODEL,
        "instructions": "Synthetic cache continuation validation; do not disclose content.",
        "input": fixed_input,
        "tools": [{
            "type": "function",
            "name": "cache_ab_probe",
            "description": "Synthetic validation function.",
            "parameters": {"type": "object", "properties": {"value": {"type": "string"}}},
        }],
        "store": True,
        "stream": False,
    }


def dry_run() -> dict[str, Any]:
    body = request_body()
    wire = safe_json(body)
    return {
        "mode": "dry-run",
        "status": "BLOCKED_CONDITIONAL",
        "reason": "安全骨架：真实 A/B 尚未实现；本次未发送业务 POST。",
        "request_hash": digest(wire),
        "request_length": len(wire),
        "input_length": len(body["input"]),
        "input_hash": digest(body["input"]),
        "model": DRY_RUN_MODEL,
        "tool_count": len(body["tools"]),
        "ab_definition": {
            "A": f"direct {A_URL}",
            "B": f"temporary proxy {B_URL}",
        },
        "planned_scenarios": [
            "warmup_200_x3",
            "growing_history",
            "function_call_new_call_id_continuation_x3",
            "connection_close_keep_alive",
        ],
        "executed_scenarios": [],
        "security": [
            "no credentials read",
            "no network I/O",
            "no 127.0.0.1:11434",
            "no 11441 business request",
            "no process started",
            "no secret value printed",
        ],
        "security_contract": {
            "network_calls": 0,
            "credentials_read": False,
            "secret_values_printed": False,
            "processes_started": 0,
            "allowed_urls": [A_URL, B_URL],
            "allowed_upstream_ports": sorted(ALLOWED_UPSTREAM_PORTS),
            "allowed_local_ports": sorted(ALLOWED_LOCAL_PORTS),
            "forbidden_local_ports": sorted(FORBIDDEN_LOCAL_PORTS),
            "forbidden_urls": sorted(FORBIDDEN_URLS),
            "production_business_requests": 0,
        },
    }


def self_test() -> int:
    """Check the dry-run contract without contacting any endpoint."""
    first = dry_run()
    assert first["executed_scenarios"] == []
    assert first["ab_definition"] == {
        "A": f"direct {A_URL}",
        "B": f"temporary proxy {B_URL}",
    }
    assert first["model"] == DRY_RUN_MODEL
    assert first["security"] == [
        "no credentials read",
        "no network I/O",
        "no 127.0.0.1:11434",
        "no 11441 business request",
        "no process started",
        "no secret value printed",
    ]
    assert first["security_contract"] == {
        "network_calls": 0,
        "credentials_read": False,
        "secret_values_printed": False,
        "processes_started": 0,
        "allowed_urls": [A_URL, B_URL],
        "allowed_upstream_ports": [11434],
        "allowed_local_ports": [11449],
        "forbidden_local_ports": [11434, 11441],
        "forbidden_urls": [
            "http://127.0.0.1:11434",
            "http://127.0.0.1:11441",
        ],
        "production_business_requests": 0,
    }
    assert first["security_contract"]["allowed_urls"] == [A_URL, B_URL]
    assert not set(first["security_contract"]["allowed_urls"]) & FORBIDDEN_URLS
    real = run_real_result()
    assert real["status"] == REAL_STATUS
    assert real["network_calls"] == 0
    assert real["credentials_read"] is False
    # The request is fixed by design; ambient model/config variables cannot alter its hash.
    assert dry_run()["request_hash"] == first["request_hash"]
    print(json.dumps({"mode": "self-test", "status": "PASS"}, sort_keys=True))
    return 0


def run_real_result() -> dict[str, Any]:
    """Describe unavailable real mode without performing side effects."""
    return {"status": REAL_STATUS, "network_calls": 0, "credentials_read": False}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run-real", action="store_true", help="reserved; real A/B is not implemented")
    parser.add_argument("--self-test", action="store_true", help="verify the no-network dry-run contract")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    if args.run_real:
        result = run_real_result()
        print(json.dumps({
            "mode": "real",
            "status": result["status"],
            "reason": "真实 A/B 场景尚未完整实现；为安全起见未读取凭证、未发送网络请求。",
            "ab_definition": {"A": A_URL, "B": B_URL},
        }, sort_keys=True))
        return 2
    print(json.dumps(dry_run(), sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
