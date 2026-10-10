"""Regression checks for the local Ghidra-export importer."""

import re
import subprocess
import tempfile
import unittest
from contextlib import closing
from pathlib import Path
from unittest.mock import patch

from db import audit, connect, index_tree, project_files, refresh


class KnowledgeDatabaseTests(unittest.TestCase):
    def test_rust_compile_time_fixtures_are_included(self):
        root = Path(__file__).resolve().parent.parent
        fixtures = set()
        for source in (root / "src").rglob("*.rs"):
            for name in re.findall(r'include_(?:str|bytes)!\(\s*"([^"]+)"', source.read_text(encoding="utf-8")):
                path = (source.parent / name).resolve()
                if path.is_relative_to(root / "analysis"):
                    self.assertTrue(path.is_file(), str(path))
                    fixtures.add(path.relative_to(root).as_posix())
        self.assertTrue(fixtures)
        indexed = {label for label, _, _ in project_files()}
        self.assertFalse(fixtures - indexed, f"Unindexed compile-time fixtures: {fixtures - indexed}")
        ignored = subprocess.run(
            ["git", "check-ignore", "--no-index", "-z", "--stdin"], cwd=root,
            input=("\0".join(sorted(fixtures)) + "\0").encode(), capture_output=True,
        )
        self.assertEqual(ignored.returncode, 1, ignored.stdout + ignored.stderr)

    def test_refresh_preserves_findings_without_reading_snapshot(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            with closing(connect(root / "test.sqlite")) as db:
                db.execute(
                    "INSERT INTO findings(id,subject,predicate,value,status,scope,limitation) "
                    "VALUES('verified','test','behavior','ported','implemented','test','')"
                )
                with patch("db.PROJECT", root), patch("db.project_files", return_value=[]), \
                        patch("db.SEED", root / "snapshot-that-must-not-be-read.json"):
                    self.assertEqual(refresh(db), (0, 0))
                    # Missing evidence should fail the DB audit, not trigger
                    # a snapshot read or a finding import.
                    self.assertFalse(audit(db))
                self.assertEqual(db.execute("SELECT value FROM findings WHERE id='verified'").fetchone()[0], "ported")

    def test_refresh_indexes_both_f8_report_folders_without_tool_caches(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            captures = []
            for name in ("collision-reports", "streaming-reports"):
                capture_dir = root / "artifacts" / name
                capture_dir.mkdir(parents=True)
                for filename in ("capture.log", "capture.png"):
                    path = capture_dir / filename
                    path.write_bytes(b"raw capture")
                    captures.append(path.relative_to(root).as_posix())
            cache = root / "artifacts" / "ghidra-tool-output"
            cache.mkdir()
            (cache / "unrelated.json").write_text("{}")
            with closing(connect(root / "test.sqlite")) as db:
                with patch("db.PROJECT", root), patch("db.project_files", return_value=[]):
                    self.assertEqual(refresh(db), (4, 4))
                    self.assertEqual(refresh(db), (0, 4))
                self.assertEqual(
                    {row[0] for row in db.execute("SELECT path FROM resources")},
                    set(captures),
                )

    def test_decompilation_manifest_import_is_repeatable(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            corpus = root / "export"
            corpus.mkdir()
            (corpus / "chunk_000000.c").write_text(
                "/* GHIDRA_FUNCTION 140001000 FUN_140001000 */\nvoid FUN_140001000(void) {}\n",
                encoding="utf-8",
            )
            (corpus / "functions.tsv").write_text(
                "address\tname\tnamespace\tbytes\tstatus\tchunk\n"
                "140001000\tFUN_140001000\tGlobal\t12\tok\tchunk_000000.c\n",
                encoding="utf-8",
            )
            (corpus / "calls.tsv").write_text(
                "caller\tcallee\n140001000\t140002000\n", encoding="utf-8"
            )
            (corpus / "types.tsv").write_text(
                "category\tname\tkind\tbytes\n/ark\tPlayer\tStructureDB\t80\n",
                encoding="utf-8",
            )
            (corpus / "type_fields.tsv").write_text(
                "category\tparent\tordinal\toffset\tbytes\tfield\ttype\n"
                "/ark\tPlayer\t0\t0\t4\thealth\tfloat\n",
                encoding="utf-8",
            )
            with closing(connect(root / "test.sqlite")) as db:
                self.assertEqual(index_tree(db, corpus, "smoke"), 1)
                self.assertEqual(index_tree(db, corpus, "smoke"), 0)
                for table in (
                    "documents", "decompiled_functions", "function_calls",
                    "decompiled_types", "decompiled_type_fields",
                ):
                    self.assertEqual(
                        db.execute(f"SELECT count(*) FROM {table}").fetchone()[0], 1, table
                    )
                self.assertEqual(db.execute("PRAGMA integrity_check").fetchone()[0], "ok")


if __name__ == "__main__":
    unittest.main()
