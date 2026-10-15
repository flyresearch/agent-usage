import importlib.util
import json
import subprocess
import sys
import tempfile
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("trial_audit", Path(__file__).parents[1] / "scripts/audit-trial.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def sample(elapsed, error=None, attempt="same", state=None):
    return {
        "elapsed_seconds": elapsed,
        "observed_at": f"2026-01-{1 + elapsed // 86400:02d}T00:00:00Z",
        "report_ok": True,
        "sum_matches": True,
        "hosts": [{"host_id": f"private-fixture-{n}", "state": state or ("stale" if error and n == 0 else "fresh"), "error": error if n == 0 else None, "last_attempt_at": attempt} for n in range(4)],
    }


class TrialAuditTests(unittest.TestCase):
    def test_partial_window_never_means_completed(self):
        result = module.audit([sample(0), sample(3600)], [], [], 24)
        self.assertFalse(result["window_elapsed"])
        self.assertTrue(result["review_required"])
        self.assertNotIn("private-fixture", json.dumps(result))

    def test_full_window_still_requires_operator_review(self):
        records = [sample(0), sample(86400)]
        records[-1]["sum_matches"] = False
        result = module.audit(records, [], [], 24)
        self.assertTrue(result["window_elapsed"])
        self.assertEqual(result["aggregate_mismatch_samples"], 1)
        self.assertTrue(result["review_required"])

    def test_same_attempt_completion_is_not_a_failed_recovery(self):
        result = module.audit([sample(0, "collection_interrupted"), sample(60)], [], [], 24)
        self.assertEqual(result["inflight_markers_followed_by_same_attempt_success"], 1)
        self.assertEqual(result["hosts_with_failure_then_recovery"], 0)
        result = module.audit([sample(0, "source_changed_during_scan"), sample(60, attempt="next")], [], [], 24)
        self.assertEqual(result["hosts_with_failure_then_recovery"], 1)
        self.assertEqual(result["hosts_with_unresolved_error_observations"], 0)

    def test_verified_label_change_preserves_physical_host_count(self):
        old, new = sample(0), sample(60)
        new["hosts"][0]["host_id"] = "renamed-fixture"
        events = [{"event": "host_labels_aligned_with_inventory", "source_paths_unchanged": True, "mapping": {"private-fixture-0": "renamed-fixture"}}]
        result = module.audit([old, new], [], events, 24)
        self.assertEqual(result["hosts_with_fresh_observations"], 4)
        events[0]["mapping"]["renamed-fixture"] = "private-fixture-0"
        with self.assertRaises(ValueError):
            module.audit([old, new], [], events, 24)

    def test_resource_limit_and_unrecognized_errors_remain_visible_without_text(self):
        resources = [{"observed_at": "2026-01-01T00:00:00Z", "processes": [{"program": "AgentUsage", "pid": 1, "cpu_percent": 0.0, "rss_kib": 600 * 1024}]}]
        result = module.audit([sample(0, "PRIVATE_BODY")], resources, [], 24)
        self.assertFalse(result["ui_rss_target_met_so_far"])
        self.assertEqual(result["error_samples_by_code"], {"other_fixed_error": 1})
        self.assertNotIn("PRIVATE_BODY", json.dumps(result))

    def test_invalid_cli_input_never_echoes_paths_or_values(self):
        script=Path(__file__).parents[1]/"scripts/audit-trial.py"
        with tempfile.TemporaryDirectory() as directory:
            missing=str(Path(directory)/"PRIVATE_SENTINEL.jsonl")
            result=subprocess.run([sys.executable,str(script),"--observations",missing,"--resources",missing,"--events",missing],capture_output=True,text=True)
            self.assertNotEqual(result.returncode,0)
            self.assertEqual(json.loads(result.stdout)["error"],"invalid_trial_input")
            self.assertEqual(result.stderr,"")
            self.assertNotIn("PRIVATE_SENTINEL",result.stdout)
        result=subprocess.run([sys.executable,str(script),"--PRIVATE_SECRET"],capture_output=True,text=True)
        self.assertNotEqual(result.returncode,0)
        self.assertEqual(result.stderr,"")
        self.assertNotIn("PRIVATE_SECRET",result.stdout)


if __name__ == "__main__":
    unittest.main()
