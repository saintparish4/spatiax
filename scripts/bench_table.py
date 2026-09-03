#!/usr/bin/env python3
"""Print the latest `cargo bench` results as the Markdown table in the README.

Reads what criterion wrote under `target/criterion/` and prints a line naming
the machine followed by one row per benchmark. Nothing here measures
anything; run `cargo bench` first. CI appends the output to the job summary
so every run's table can be read without downloading artifacts.
"""

import datetime
import json
import os
import platform
import subprocess
import sys
from pathlib import Path

RESULTS = Path("target/criterion")

# What one element of throughput is, per benchmark group.
UNITS = {
    "decode_frame": "frame",
    "decode_message": "signal",
    "extract_raw": "signal",
    "candump": "line",
}


def cpu_model():
    try:
        with open("/proc/cpuinfo") as f:
            for line in f:
                if line.startswith("model name"):
                    return line.split(":", 1)[1].strip()
    except OSError:
        pass
    return platform.processor() or "unknown CPU"


def rustc_version():
    try:
        return subprocess.run(
            ["rustc", "--version"], capture_output=True, text=True, check=True
        ).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return "rustc (version unknown)"


def machine_line():
    return (
        f"Measured on {cpu_model()} ({os.cpu_count()} threads), "
        f"{platform.system()} {platform.release()}, {rustc_version()}, "
        f"{datetime.date.today().isoformat()}."
    )


def nanoseconds(ns):
    if ns >= 1_000_000:
        return f"{ns / 1_000_000:.2f} ms"
    if ns >= 1_000:
        return f"{ns / 1_000:.2f} µs"
    return f"{ns:.1f} ns"


def rate(per_second, unit):
    if per_second >= 1_000_000:
        return f"{per_second / 1_000_000:.1f} M {unit}s/s"
    if per_second >= 1_000:
        return f"{per_second / 1_000:.1f} k {unit}s/s"
    return f"{per_second:.0f} {unit}s/s"


def load_results():
    # criterion writes each benchmark's files as it finishes, so ordering by
    # modification time reproduces the order of the bench source.
    files = sorted(RESULTS.rglob("new/benchmark.json"), key=lambda p: p.stat().st_mtime)
    for benchmark in files:
        with open(benchmark) as f:
            info = json.load(f)
        with open(benchmark.with_name("estimates.json")) as f:
            estimates = json.load(f)
        yield info, estimates["mean"]


def row(info, mean):
    group = info["group_id"]
    unit = UNITS.get(group, "element")
    elements = (info.get("throughput") or {}).get("Elements", 1)
    point = mean["point_estimate"]
    interval = mean["confidence_interval"]
    per_unit = point / elements

    name = f"`{info['full_id']}`"
    if elements != 1:
        name += f" · {elements} {unit}s"
    spread = f"{nanoseconds(point)} ({nanoseconds(interval['lower_bound'])} – {nanoseconds(interval['upper_bound'])})"
    return f"| {name} | {spread} | {nanoseconds(per_unit)} / {unit} | {rate(1e9 / per_unit, unit)} |"


def main():
    results = list(load_results())
    if not results:
        sys.exit(f"no results under {RESULTS}; run `cargo bench` first")

    print(machine_line())
    print()
    print("| Benchmark | Mean per iteration (95% CI) | Per unit | Rate |")
    print("|---|---|---|---|")
    for info, mean in results:
        print(row(info, mean))


if __name__ == "__main__":
    main()
