"""Read-only bounded qualification; exports only numeric projections.

Independent oracle: response-identity usage plus legacy prefix for modern Codex,
monotonic total/last selection for legacy Codex, field-max for Claude samples.
It is not an acceptance test for all resets, forks, or ambiguous streaming cases.
"""
import argparse
import collections
import json
import pathlib
import subprocess
import time
import uuid


def codex_fields(u):
    return [u.get("input_tokens", 0),
            u.get("cached_input_tokens", u.get("cache_read_input_tokens", 0)),
            u.get("output_tokens", 0), u.get("reasoning_output_tokens", 0), u.get("cache_write_input_tokens", 0)]


def canonical(a):
    return [a[0] - a[1] - a[4], a[1], a[4], a[2] - a[3], a[3]]


def add(a, b):
    return [x + y for x, y in zip(a, b)]


def legacy_oracle(rows):
    total, previous, seen = [0] * 5, None, set()
    for row in rows:
        if row["type"] != "event_msg":
            continue
        info = row["info"]
        current = info.get("total_token_usage")
        last = info.get("last_token_usage")
        identity = json.dumps((row["timestamp"], current, last), sort_keys=True)
        if identity in seen:
            continue
        seen.add(identity)
        current = codex_fields(current) if current else None
        if current is not None and current == previous:
            continue
        if current is not None and previous is not None and any(x < y for x, y in zip(current, previous)) and not last:
            return None
        increment = codex_fields(last) if last else (
            [x - y for x, y in zip(current, previous)] if previous else current)
        if increment is None:
            return None
        if not any(increment):
            continue
        if current is not None:
            previous = current
        elif previous:
            previous = add(previous, increment)
        total = add(total, canonical(increment))
    return total


def oracle(path, adapter):
    rows, messages = [], {}
    first_meta = True
    forked = False
    owner = None
    current_turn = None
    with path.open("rb") as source:
        for raw in source:
            if not raw.endswith(b"\n"):
                break
            v = json.loads(raw)
            if adapter == "codex-jsonl":
                p = v.get("payload") or {}
                if first_meta and v.get("type") == "session_meta":
                    first_meta = False
                    forked = lineage_marked(p)
                    owner = p.get("id")
                if v.get("type") == "turn_context":
                    current_turn = p.get("turn_id")
                if v.get("type") == "token_usage_record":
                    rows.append({"type": "token_usage_record", "identity": (p["thread_id"], p["response_id"]), "usage": p["usage"]})
                elif v.get("type") == "event_msg" and p.get("type") == "token_count" and p.get("info"):
                    rows.append({"type": "event_msg", "timestamp": v.get("timestamp"), "info": p["info"], "turn_id": current_turn})
            else:
                m = v.get("message") or {}
                u = m.get("usage")
                if v.get("type") != "assistant" or not u:
                    continue
                key = (m.get("id"), v.get("requestId"))
                counts = [u.get(k, 0) for k in ["input_tokens", "cache_read_input_tokens", "cache_creation_input_tokens", "output_tokens"]] + [0]
                messages[key] = [max(x, y) for x, y in zip(messages.get(key, [0] * 5), counts)]
    if adapter == "claude-jsonl":
        return [sum(r[i] for r in messages.values()) for i in range(5)], None
    legacy = legacy_oracle(rows)
    first = next((i for i, r in enumerate(rows) if r["type"] == "token_usage_record"), None)
    if first is None:
        if not forked:
            return legacy, None
        def millis(value):
            try:
                parsed = uuid.UUID(value)
                return parsed.int >> 80 if parsed.version == 7 else None
            except (ValueError, TypeError, AttributeError):
                return None
        created = millis(owner)
        first_own = next((i for i,r in enumerate(rows) if created is not None and millis(r.get("turn_id")) is not None and millis(r["turn_id"]) > created), None)
        if first_own is None or legacy is None:
            return None, None
        prefix = legacy_oracle(rows[:first_own])
        return ([a-b for a,b in zip(legacy,prefix)] if prefix is not None else None), None
    selected = [0] * 5 if forked else legacy_oracle(rows[:first])
    if selected is None:
        return None, None
    responses = {}
    for row in rows[first:]:
        if row["type"] == "token_usage_record":
            responses[row["identity"]] = canonical(codex_fields(row["usage"]))
    for counts in responses.values():
        selected = add(selected, counts)
    return selected, sum(selected) - sum(legacy) if legacy is not None and not forked else None


def lineage_marked(payload):
    nested = payload
    for key in ["source", "subagent", "thread_spawn"]:
        nested = nested.get(key, {}) if isinstance(nested, dict) else {}
    parent = nested.get("parent_thread_id") if isinstance(nested, dict) else None
    return bool(payload.get("forked_from_id") or parent)


def fork_file(path):
    with path.open("rb") as source:
        line = source.readline()
    if not line.endswith(b"\n"):
        return False
    row = json.loads(line)
    return row.get("type") == "session_meta" and lineage_marked(row.get("payload") or {})


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("adapter", choices=["codex-jsonl", "claude-jsonl"])
    cli.add_argument("root", type=pathlib.Path)
    cli.add_argument("--limit", type=int, default=12)
    cli.add_argument("--fork-only", action="store_true", help="Select Codex files with explicit fork/parent metadata")
    args = cli.parse_args()
    if args.fork_only and args.adapter != "codex-jsonl":
        raise SystemExit("fork_filter_requires_codex")
    if not 1 <= args.limit <= 100:
        raise SystemExit("invalid_sample_limit")
    candidates = args.root.rglob("*.jsonl")
    if args.fork_only:
        candidates = (p for p in candidates if fork_file(p))
    paths = sorted(candidates, key=lambda p: p.stat().st_mtime, reverse=True)[:args.limit]
    if not paths:
        raise SystemExit("no_samples")
    executable = pathlib.Path.cwd() / "target/debug/examples/parse_file"
    report = dict(adapter=args.adapter, samples=len(paths), matched=0,
                  errors=collections.Counter(), oracle_unqualified=0, changed=0,
                  bytes=0, seconds=0, total_tokens=0,
                  response_vs_legacy_samples=0, response_vs_legacy_delta_tokens=0)
    for path in paths:
        before = path.stat()
        start = time.monotonic()
        rust = json.loads(subprocess.check_output([str(executable), args.adapter, str(path)]))
        report["seconds"] += time.monotonic() - start
        report["bytes"] += before.st_size
        if "error" in rust:
            report["errors"][rust["error"]] += 1
            continue
        if rust["coverage"].get("unmetered_files", 0) or rust["coverage"].get("invalid_cache_ttl_records", 0):
            report["oracle_unqualified"] += 1
            continue
        expected, legacy_delta = oracle(path, args.adapter)
        after = path.stat()
        if (before.st_size, before.st_mtime_ns, before.st_ctime_ns) != (after.st_size, after.st_mtime_ns, after.st_ctime_ns):
            report["changed"] += 1
            continue
        if expected is None:
            report["oracle_unqualified"] += 1
            continue
        actual = [rust["tokens"][k] for k in ["input", "cache_read", "cache_write", "output", "reasoning"]]
        if actual != expected:
            report["errors"]["comparison_mismatch"] += 1
        else:
            report["matched"] += 1
            report["total_tokens"] += sum(expected)
            if legacy_delta is not None:
                report["response_vs_legacy_samples"] += 1
                report["response_vs_legacy_delta_tokens"] += legacy_delta
    print(json.dumps(report))


if __name__ == "__main__":
    try:
        main()
    except Exception:
        raise SystemExit("qualification_failed") from None
