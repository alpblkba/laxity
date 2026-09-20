#!/usr/bin/env python3
"""boardless checks for the audit model, provenance labels and search ceiling."""

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
LAXITY = ROOT / "tools/laxity"
MAP = ROOT / "build/target/laxity-u585.map"
PROFILE = ROOT / "profiles/stm32u585.toml"


def run_audit(config):
    return subprocess.run(
        [sys.executable, str(LAXITY), "audit", str(config)],
        cwd=ROOT,
        check=False,
        capture_output=True,
        text=True,
    )


def config_text(characterisation, objects=1):
    declarations = []
    for index in range(objects):
        declarations.append(
            """
[[object]]
workload = "inference"
name = "stack%d"
symbol = "tx_byte_pool_buffer"
size = 1
region = "sram3"
movable = %s
regions = ["sram1", "sram2", "sram3", "sram4"]
""" % (index, "true" if objects > 1 else "false")
        )
    return """\
schema_version = 1
platform = "stm32u585"
profile = "%s"
characterisation = "%s"
map = "%s"

[workload.inference]
window_cycles = 320000
deadline_cycles = 5120000
%s
[[requester]]
name = "dma"
endpoint = "measured"
region = "sram3"
transactions_per_second = 12800000

[[requester]]
name = "dma"
endpoint = "borrowed"
region = "sram3"
transactions_per_second = 12800000

[[requester]]
name = "radio"
endpoint = "unknown"
region = "sram3"
""" % (PROFILE, characterisation, MAP, "".join(declarations))


def characterisation_text(bad_borrow=False):
    borrowed = "value = 0.8" if bad_borrow else "minimum = 0.08\nmaximum = 0.8"
    return """\
schema_version = 1
platform = "stm32u585"
date = "2026-09-20"
image_sha256 = "0000000000000000000000000000000000000000000000000000000000000000"
resolution = "region"
captures = ["fixture"]

[[coefficient]]
object = "stack0"
requester = "dma"
endpoint = "measured"
basis = "measured"
value = 0.1

[[coefficient]]
object = "stack0"
requester = "dma"
endpoint = "borrowed"
basis = "borrowed"
borrowed_from = "other-mcu"
%s

[[coefficient]]
object = "stack0"
requester = "radio"
endpoint = "unknown"
basis = "unmeasured"
""" % borrowed


class AuditTests(unittest.TestCase):
    def test_reference_is_boardless_and_enumerates_the_declared_space(self):
        result = run_audit(ROOT / "examples/stm32u585-reference/laxity.toml")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("laxity audit  ·  build/target/laxity-u585.elf", result.stdout)
        self.assertIn("48 assignments enumerated", result.stdout)
        self.assertIn("placement below region granularity is not", result.stdout)
        self.assertNotIn("ST-LINK", result.stdout + result.stderr)

    def test_all_three_provenance_labels_are_visibly_distinct(self):
        with tempfile.TemporaryDirectory(prefix="laxity-audit-") as directory:
            temp = Path(directory)
            characterisation = temp / "characterisation.toml"
            config = temp / "laxity.toml"
            characterisation.write_text(characterisation_text())
            config.write_text(config_text(characterisation))

            result = run_audit(config)

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("measured, this platform", result.stdout)
        self.assertIn("borrowed from other-mcu, order of magnitude only", result.stdout)
        self.assertIn("unmeasured", result.stdout)
        self.assertIn("0.08 .. 0.8 /xact", result.stdout)

    def test_borrowed_coefficient_cannot_be_a_point_estimate(self):
        with tempfile.TemporaryDirectory(prefix="laxity-audit-") as directory:
            temp = Path(directory)
            characterisation = temp / "characterisation.toml"
            config = temp / "laxity.toml"
            characterisation.write_text(characterisation_text(bad_borrow=True))
            config.write_text(config_text(characterisation))

            result = run_audit(config)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("borrowed coefficient", result.stderr)

    def test_search_larger_than_ten_thousand_is_not_called_optimal(self):
        with tempfile.TemporaryDirectory(prefix="laxity-audit-") as directory:
            temp = Path(directory)
            characterisation = temp / "characterisation.toml"
            config = temp / "laxity.toml"
            characterisation.write_text(
                "schema_version = 1\nplatform = \"stm32u585\"\ndate = \"2026-09-20\"\n"
                "image_sha256 = \"0000000000000000000000000000000000000000000000000000000000000000\"\n"
                "resolution = \"region\"\ncaptures = [\"fixture\"]\n"
            )
            config.write_text(config_text(characterisation, objects=7))

            result = run_audit(config)

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("search truncated", result.stdout)
        self.assertNotIn("constraints.  best, and the runner up", result.stdout)


if __name__ == "__main__":
    unittest.main()
