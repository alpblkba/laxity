#!/usr/bin/env python3
"""Check the restricted reader against the standard parser and the CLI import order."""

import builtins
import contextlib
import io
import runpy
import sys
import tempfile
import types
import unittest
from pathlib import Path
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))
import laxity_toml

try:
    import tomllib
except ModuleNotFoundError:
    tomllib = None

read_toml = runpy.run_path(str(ROOT / "tools/laxity"))["read_toml"]


class TomlTests(unittest.TestCase):
    @unittest.skipIf(tomllib is None, "tomllib is unavailable; repository parity needs Python 3.11 or newer")
    def test_every_repository_toml_matches_tomllib(self):
        paths = sorted(ROOT.rglob("*.toml"))
        self.assertTrue(paths, "no TOML files found")
        for path in paths:
            with self.subTest(path=str(path.relative_to(ROOT))):
                text = path.read_bytes().decode("utf-8")
                self.assertEqual(laxity_toml.loads(text), tomllib.loads(text))
        print("TOML parity checked %d repository files" % len(paths))

    def test_values_tables_and_comments_keep_their_meaning(self):
        text = ('title = "a # b, = [] {}" # comment\r\n'
                'enabled = true\nempty = []\n'
                'items = [\n"one", # first\n"two",\n]\n'
                'options = { flag = false, features = ["read", "std"] }\n'
                '[workload.inference]\ncount = 42\nratio = 0.020\naddress = 0x20000000\n'
                '[workload]\nname = "inference"\n'
                '[[object]]\nname = "first"\n[[object]]\nname = "second"')
        expected = {"title": "a # b, = [] {}", "enabled": True, "empty": [],
                    "items": ["one", "two"], "options": {"flag": False, "features": ["read", "std"]},
                    "workload": {"inference": {"count": 42, "ratio": 0.02, "address": 0x20000000},
                                 "name": "inference"},
                    "object": [{"name": "first"}, {"name": "second"}]}
        self.assertEqual(laxity_toml.loads(text), expected)
        if tomllib is not None:
            self.assertEqual(tomllib.loads(text), expected)

    def test_unsupported_and_invalid_syntax_has_a_line_number(self):
        unsupported = [
            '"key" = 1', 'a.b = 1', "x = 'literal'", 'x = """multiline"""',
            r'x = "escape\n"', 'x = -1', 'x = +1', 'x = 1_000', 'x = 0b10', 'x = 0o10',
            'x = 1e2', 'x = nan', 'x = inf', 'x = 2026-09-26', 'x = 01', 'x = 1.',
            'x = [[1]]', 'x = [{ a = 1 }]', 'x = { a = { b = 1 } }',
            'x = { a = 1, }', 'x = { a = 1, a = 2 }', 'x = [1 2]', 'x = "a" junk',
            'x = "unterminated', 'x = "a\x00b"', 'x = 1\r', '# bad\x7f',
            '[[nested.tables]]', '["quoted"]', '[a . b]', 'ok = 2', 'x = '
        ]
        for text in unsupported:
            with self.subTest(text=text), self.assertRaisesRegex(ValueError, r"line 2:"):
                laxity_toml.loads("ok = 1\n" + text)
        for text in ['[a]\n[a]', 'a = {}\n[a.b]', 'a = []\n[[a]]',
                     '[[a]]\n[a.b]', '[a]\n[[a]]', 'a = 1\n[a]']:
            with self.subTest(text=text), self.assertRaisesRegex(ValueError, r"line 2:"):
                laxity_toml.loads(text)
        for text, line in [('x = [\n1,\n', 3), ('x = {\na = 1\n}', 1),
                           ('x = { a = [\n1\n] }', 1)]:
            with self.subTest(text=text), self.assertRaisesRegex(ValueError, "line %d:" % line):
                laxity_toml.loads(text)

    def test_cli_reports_the_fallback_error_with_its_path_and_line(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "invalid.toml"
            path.write_text("ok = 1\nx = 'unsupported'", encoding="utf-8")
            errors = io.StringIO()
            with patch.dict(sys.modules, {"tomllib": None, "tomli": None}), \
                    contextlib.redirect_stderr(errors), self.assertRaises(SystemExit):
                read_toml(path)
            self.assertIn(str(path), errors.getvalue())
            self.assertIn("line 2:", errors.getvalue())

    def test_cli_tries_standard_optional_then_local_parser(self):
        original_import = builtins.__import__
        for available, wanted in [("tomllib", ["tomllib"]),
                                  ("tomli", ["tomllib", "tomli"]),
                                  (None, ["tomllib", "tomli", "laxity_toml"])]:
            seen = []
            optional = types.SimpleNamespace(loads=lambda text: {"selected": available})

            def importing(name, *args, **kwargs):
                if name in ("tomllib", "tomli", "laxity_toml"):
                    seen.append(name)
                    if name == available:
                        return optional
                    if name != "laxity_toml":
                        raise ModuleNotFoundError(name, name=name)
                return original_import(name, *args, **kwargs)

            with self.subTest(available=available), tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / "input.toml"
                path.write_text('selected = "local"', encoding="utf-8")
                with patch("builtins.__import__", side_effect=importing):
                    parsed = read_toml(path)
                self.assertEqual(parsed, {"selected": available or "local"})
                self.assertEqual(seen, wanted)


if __name__ == "__main__":
    unittest.main(verbosity=2)
