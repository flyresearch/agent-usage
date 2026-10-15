#!/usr/bin/env python3
"""Audit private trial observations without printing host IDs or token counts.

This summarizes evidence; it does not mark the release goal complete. Process
completion and any observation gaps still need operator review.
"""
import argparse
import collections
import datetime as dt
import json
import math
import sys
from pathlib import Path


def load_lines(path):
    with path.open(encoding="utf-8") as stream:
        return [json.loads(line) for line in stream if line.strip()]


def instant(text):
    value = dt.datetime.fromisoformat(text.replace("Z", "+00:00"))
    if value.tzinfo is None:
        raise ValueError("missing_timezone")
    return value


def audit(observations, resources, events, minimum_hours):
    if not observations:
        raise ValueError("missing_observations")
    aliases = {}
    for event in events:
        if event.get("event") == "host_labels_aligned_with_inventory":
            if event.get("source_paths_unchanged") is not True:
                raise ValueError("unverified_host_mapping")
            aliases.update(event["mapping"])

    def identity(host):
        seen = set()
        while host in aliases:
            if host in seen:
                raise ValueError("cyclic_host_mapping")
            seen.add(host)
            host = aliases[host]
        return host

    elapsed = [record["elapsed_seconds"] for record in observations]
    if any(type(value) not in (int, float) or not math.isfinite(value) or value < 0 for value in elapsed):
        raise ValueError("invalid_elapsed_time")
    if elapsed != sorted(elapsed):
        raise ValueError("nonmonotonic_observations")
    times = [instant(record["observed_at"]) for record in observations]
    gaps = [(b - a).total_seconds() for a, b in zip(times, times[1:])]
    failures = 0
    bad_sums = 0
    bad_host_counts = 0
    errors = collections.Counter()
    fresh_hosts = set()
    pending = {}
    recovered_hosts = set()
    completed_inflight = 0
    for record in observations:
        if record.get("report_ok") is not True:
            failures += 1
            continue
        if record.get("sum_matches") is not True:
            bad_sums += 1
        hosts = record.get("hosts", [])
        host_ids = [identity(host["host_id"]) for host in hosts]
        if len(hosts) != 4 or len(set(host_ids)) != 4:
            bad_host_counts += 1
        for host, host_id in zip(hosts, host_ids):
            error = host.get("error")
            if error:
                # Do not echo arbitrary strings from a supplied observation.
                known = {"collection_interrupted", "source_changed_during_scan", "collection_busy", "collection_timeout", "collection_failed", "source_missing", "source_unreadable", "fleet_snapshot_identity_mismatch"}
                errors[error if error in known else "other_fixed_error"] += 1
                pending[host_id] = (error, host.get("last_attempt_at"))
            if host.get("state") == "fresh":
                fresh_hosts.add(host_id)
                if host_id in pending:
                    prior_error, attempt = pending.pop(host_id)
                    if prior_error == "collection_interrupted" and attempt == host.get("last_attempt_at"):
                        completed_inflight += 1
                    else:
                        recovered_hosts.add(host_id)

    ui_by_pid = collections.defaultdict(list)
    for record in resources:
        timestamp = instant(record["observed_at"])
        for process in record.get("processes", []):
            if process.get("program") == "AgentUsage":
                cpu, rss = process["cpu_percent"], process["rss_kib"]
                if not math.isfinite(cpu) or cpu < 0 or rss < 0:
                    raise ValueError("invalid_resource_sample")
                ui_by_pid[process["pid"]].append((timestamp, cpu, rss))
    all_ui = [value for values in ui_by_pid.values() for value in values]
    recent = sorted(all_ui)[-12:]
    peak = max((value[2] for value in all_ui), default=None)
    duration = elapsed[-1] - elapsed[0]
    return {
        "kind": "trial_audit",
        "elapsed_hours": round(duration / 3600, 3),
        "required_hours": minimum_hours,
        "window_elapsed": duration >= minimum_hours * 3600,
        "samples": len(observations),
        "latest_report_ok": observations[-1].get("report_ok") is True,
        "latest_complete": observations[-1].get("complete"),
        "latest_state_counts": dict(collections.Counter(host.get("state") if host.get("state") in {"fresh","stale","never_collected","configuration_changed"} else "unrecognized" for host in observations[-1].get("hosts",[]))),
        "maximum_observation_gap_seconds": max(gaps, default=0),
        "clock_rollback_observed": any(gap < 0 for gap in gaps),
        "report_failure_samples": failures,
        "aggregate_mismatch_samples": bad_sums,
        "unexpected_host_count_samples": bad_host_counts,
        "hosts_with_fresh_observations": len(fresh_hosts),
        "error_samples_by_code": dict(errors),
        "inflight_markers_followed_by_same_attempt_success": completed_inflight,
        "hosts_with_failure_then_recovery": len(recovered_hosts),
        "hosts_with_unresolved_error_observations": len(pending),
        "resource_samples": len(resources),
        "ui_process_lifetimes_observed": len(ui_by_pid),
        "ui_peak_rss_kib": peak,
        "ui_rss_target_met_so_far": None if peak is None else peak <= 512 * 1024,
        "recent_ui_cpu_percent": [value[1] for value in recent],
        "review_required": True,
    }


def main():
    if sys.argv[1:] == ["--help"]:
        print(json.dumps({"kind":"help","commands":["audit-trial.py --observations PRIVATE_JSONL --resources PRIVATE_JSONL --events PRIVATE_JSONL [--minimum-hours 24]"]}))
        return
    class Parser(argparse.ArgumentParser):
        def error(self, message):
            raise ValueError("invalid_trial_arguments")
    parser = Parser(add_help=False)
    parser.add_argument("--observations", type=Path, required=True)
    parser.add_argument("--resources", type=Path, required=True)
    parser.add_argument("--events", type=Path, required=True)
    parser.add_argument("--minimum-hours", type=float, default=24)
    args = parser.parse_args()
    if not math.isfinite(args.minimum_hours) or args.minimum_hours <= 0:
        raise ValueError("invalid_window")
    result = audit(load_lines(args.observations), load_lines(args.resources), load_lines(args.events), args.minimum_hours)
    print(json.dumps(result, sort_keys=True))


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError, OverflowError):
        print(json.dumps({"kind": "trial_audit", "error": "invalid_trial_input", "review_required": True}))
        raise SystemExit(1)
