#!/usr/bin/env python3
"""Measure the provider-free model-facing runtime contract.

Combines the serialized tool catalog and the rendered system prompt into a
single reproducible receipt. No API keys or live providers are required.
"""

from __future__ import annotations

import json
import subprocess
import sys


def run_metric(test_name: str, marker: str) -> dict:
    cmd = [
        "cargo",
        "test",
        "-p",
        "codewhale-tui",
        test_name,
        "--",
        "--ignored",
        "--nocapture",
        "--test-threads=1",
    ]
    proc = subprocess.run(cmd, text=True, capture_output=True, check=False)
    sys.stderr.write(proc.stderr)

    combined = proc.stdout.splitlines() + proc.stderr.splitlines()
    for line in combined:
        if marker in line:
            return json.loads(line.split(marker, 1)[1])

    sys.stdout.write(proc.stdout)
    raise RuntimeError(f"missing {marker} marker")


def main() -> int:
    tool_metrics = run_metric(
        "print_agent_tool_catalog_metrics",
        "TOOL_CATALOG_METRICS ",
    )
    prompt_metrics = run_metric(
        "print_agent_runtime_contract_metrics",
        "RUNTIME_CONTRACT_METRICS ",
    )

    receipt = {
        "tool_catalog": tool_metrics,
        "system_prompt": prompt_metrics,
    }
    print(json.dumps(receipt, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
