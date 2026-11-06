"""Summarize the opt-in Telegram matrix without counting duplicates or partial samples."""
import argparse
import json
import math
from collections import defaultdict
from pathlib import Path


def percentile(values, fraction):
    ordered = sorted(values)
    if not ordered:
        return None
    position = (len(ordered) - 1) * fraction
    left = math.floor(position)
    right = math.ceil(position)
    return ordered[left] + (ordered[right] - ordered[left]) * (position - left)


def size_label(size):
    whole, remainder = divmod(size, 1048576)
    if not remainder:
        return f"{whole} MiB"
    if not whole:
        return f"{size / 1048576:g} MiB"
    return f"{whole} MiB + {remainder} B"


def summarize(root):
    grouped = defaultdict(list)
    seen = set()
    for line in (root / "samples.redacted.ndjson").read_text(encoding="utf-8-sig").splitlines():
        row = json.loads(line)
        key = row["sample"]
        if key in seen:
            raise ValueError(f"duplicate sample: {key}")
        seen.add(key)
        if row["measurements"]["sha256_match"] is not True:
            raise ValueError(f"unverified sample: {key}")
        grouped[(row["bytes"], row["mode"])].append(row)
    process = defaultdict(list)
    monitor = root / "process-metrics.redacted.ndjson"
    if monitor.exists():
        for line in monitor.read_text(encoding="utf-8-sig").splitlines():
            row = json.loads(line)
            process[row["sample"]].append(row)
    result = []
    for (size, mode), rows in sorted(grouped.items()):
        entry = {"bytes": size, "mode": mode, "samples": len(rows)}
        samples = [point for row in rows for point in process[row["sample"]]]
        for metric in ["working_set", "private_bytes", "tcp_established"]:
            values = [point[metric] for point in samples if point.get(metric) is not None]
            entry[metric + "_peak"] = max(values) if values else None
        for direction in ["upload", "download"]:
            values = [row["measurements"][direction] for row in rows]
            for value in values:
                if value["bytes"] != size or not math.isfinite(value["seconds"]) or value["seconds"] <= 0:
                    raise ValueError("invalid measurement size or duration")
            times = [value["seconds"] for value in values]
            speeds = [size / 1048576 / seconds for seconds in times]
            attempts = [value["attempts"] for value in values]
            failures = [value["failed_attempts"] for value in values]
            retries = [value["retries"] for value in values]
            peaks = [value["peak_rpc"] for value in values]
            entry[direction] = {
                "seconds_p50": percentile(times, 0.5),
                "seconds_p95": percentile(times, 0.95),
                "mib_per_second_p50": percentile(speeds, 0.5),
                "mib_per_second_p95": percentile(speeds, 0.95),
                "aggregate_mib_per_second": len(values) * size / 1048576 / sum(times),
                "failure_rate": sum(failures) / sum(attempts) if all(v is not None for v in failures + attempts) and sum(attempts) else None,
                "retries": sum(retries) if all(v is not None for v in retries) else None,
                "peak_rpc": max(peaks) if all(v is not None for v in peaks) else None,
            }
        result.append(entry)
    sizes = [524288, 3145728, 10485760, 10485777, 104857600, 524288000]
    modes = ["legacy", "1", "2", "4", "8", "16"]
    complete = all(len(grouped[(size, mode)]) >= 10 for size in sizes for mode in modes)
    return {"matrix_complete": complete, "valid_samples": len(seen), "groups": result}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--require-complete", action="store_true")
    args = parser.parse_args()
    result = summarize(args.directory)
    (args.directory / "summary.redacted.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
    print("| size | mode | n | upload P50/P95 s | download P50/P95 s | upload/download P50 MiB/s | TCP peak |")
    print("| ---: | --- | ---: | ---: | ---: | ---: | ---: |")
    for row in result["groups"]:
        up, down = row["upload"], row["download"]
        print(f'| {size_label(row["bytes"])} | {row["mode"]} | {row["samples"]} | '
              f'{up["seconds_p50"]:.3f}/{up["seconds_p95"]:.3f} | '
              f'{down["seconds_p50"]:.3f}/{down["seconds_p95"]:.3f} | '
              f'{up["mib_per_second_p50"]:.3f}/{down["mib_per_second_p50"]:.3f} | {row["tcp_established_peak"]} |')
    print(f'matrix_complete={result["matrix_complete"]}; valid_samples={result["valid_samples"]}')
    if args.require_complete and not result["matrix_complete"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
