"""Local, evidence-linked SQLite index for the DH2 runtime project.

Only metadata is stored for extracted game assets. Source text is indexed for
project-owned files, or for an explicitly supplied decompilation directory.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sqlite3
import sys
from pathlib import Path
import function_register as functions


PROJECT = Path(__file__).resolve().parent.parent
DEFAULT_DB = PROJECT / "knowledge" / "dh2.sqlite"
SCHEMA = Path(__file__).with_name("schema.sql")
SEED = Path(__file__).with_name("seed.json")
TEXT_SUFFIXES = {".rs", ".md", ".toml", ".sql", ".py", ".ps1", ".json", ".java", ".c", ".h", ".cpp", ".hpp", ".wgsl"}
GAME_SUFFIXES = {".bwm", ".bmodel", ".bphysworld", ".entities", ".decl", ".bimage7"}

def read_evidence(db, item):
    path = item['path']
    start, limit = int(item.get('start_line', 1)), int(item.get('max_lines', 80))
    if start < 1 or not 1 <= limit <= 200:
        raise ValueError('Invalid evidence line window')
    row = db.execute('SELECT payload,encoding,sha256 FROM stored_evidence WHERE resource_path=?', (path,)).fetchone()
    if row:
        data, encoding, sha = row
        if hashlib.sha256(data).hexdigest() != sha:
            raise ValueError('Evidence checksum mismatch')
        text = data.decode(encoding)
    else:
        cached = db.execute('SELECT content FROM documents WHERE path=?', (path,)).fetchone()
        if not cached:
            raise ValueError(f'No SQLite evidence: {path}')
        text, sha = cached[0], None
    lines = text.splitlines()
    selected = lines[start - 1:start - 1 + limit]
    return {'path':path, 'storage':'sqlite', 'sha256':sha, 'start_line':start,
            'total_lines':len(lines), 'shown_lines':len(selected),
            'has_more':start - 1 + len(selected) < len(lines),
            'next_start_line':start + len(selected), 'content':'\n'.join(selected)}


def load_capture_text(file, project=PROJECT):
    file = Path(file).resolve()
    if file == Path(project) / 'knowledge/seed.json':
        raise ValueError('Do not read seed.json')
    if file.is_file():
        return file.read_text(encoding='utf-8')
    label = file.relative_to(project).as_posix()
    db = sqlite3.connect((Path(project)/'knowledge/dh2.sqlite').as_uri()+'?mode=ro', uri=True)
    try:
        row = db.execute('SELECT payload,encoding FROM stored_evidence WHERE resource_path=?', (label,)).fetchone()
        if not row:
            raise FileNotFoundError(file)
        return row[0].decode(row[1])
    finally:
        db.close()
SYMBOL_RE = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(fn|struct|enum|trait|type|const)\s+([A-Za-z_][A-Za-z_0-9]*)\b")


def digest(path: Path) -> str:
    hasher = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            hasher.update(chunk)
    return hasher.hexdigest()


def connect(path: Path) -> sqlite3.Connection:
    path.parent.mkdir(parents=True, exist_ok=True)
    db = sqlite3.connect(path, timeout=120)
    db.execute("PRAGMA foreign_keys = ON")
    db.executescript(SCHEMA.read_text(encoding="utf-8"))
    if "confirmed" not in {row[1] for row in db.execute("PRAGMA table_info(function_inspections)")}:
        db.execute("ALTER TABLE function_inspections ADD COLUMN confirmed INTEGER NOT NULL DEFAULT 1 CHECK (confirmed IN (0,1))")
    return db


def upsert_resource(db: sqlite3.Connection, label: str, path: Path, kind: str, origin: str) -> bool:
    size = path.stat().st_size
    sha = digest(path)
    old = db.execute("SELECT sha256 FROM resources WHERE path=?", (label,)).fetchone()
    if old and old[0] == sha:
        return False
    db.execute(
        """INSERT INTO resources(path,kind,origin,byte_size,sha256)
           VALUES(?,?,?,?,?) ON CONFLICT(path) DO UPDATE SET
           kind=excluded.kind,origin=excluded.origin,byte_size=excluded.byte_size,
           sha256=excluded.sha256,indexed_at=CURRENT_TIMESTAMP""",
        (label, kind, origin, size, sha),
    )
    return True


def index_text(db: sqlite3.Connection, label: str, path: Path, origin: str) -> bool:
    if not upsert_resource(db, label, path, "source" if path.suffix != ".md" else "note", origin):
        return False
    content = path.read_text(encoding="utf-8", errors="replace")
    db.execute(
        "INSERT INTO documents(path,content) VALUES(?,?) ON CONFLICT(path) DO UPDATE SET content=excluded.content",
        (label, content),
    )
    db.execute("DELETE FROM documents_fts WHERE path=?", (label,))
    db.execute("INSERT INTO documents_fts(path,content) VALUES(?,?)", (label, content))
    db.execute("DELETE FROM symbols WHERE path=?", (label,))
    if path.suffix == ".rs":
        db.executemany(
            "INSERT INTO symbols(path,line,kind,name) VALUES(?,?,?,?)",
            ((label, number, match[1], match[2])
             for number, line in enumerate(content.splitlines(), 1)
             if (match := SYMBOL_RE.match(line))),
        )
    return True


def project_files() -> list[tuple[str, Path, str]]:
    files = [PROJECT / name for name in ("README.md", "Cargo.toml")]
    files += [PROJECT / "analysis" / name for name in (
        "probe_original_active_planes.py", "original_active_plane_cases.txt",
        "probe_original_portal_raster.py", "original_portal_small_cases.txt",
        "original_portal_strip_cases.txt", "original_portal_medium_cases.txt",
        "original_portal_large_cases.txt", "original_portal_large_varied_cases.txt",
        "probe_original_depth_grid.py",
        "original_depth_grid_cases.txt", "probe_original_depth_raster.py",
        "original_depth_raster_cases.txt", "probe_original_mask_depth.py",
        "original_mask_depth_cases.txt", "probe_original_box_clip.py",
        "original_box_clip_cases.txt", "probe_original_query_jobs.py",
        "original_query_job_cases.txt", "original_query_state_cases.txt",
        "original_spatial_query_cases.txt",
        "probe_original_object_distance.py", "original_object_distance_cases.txt",
        "probe_original_visible_cells.py", "original_visible_cell_cases.txt",
        "original_visible_cell_cases.point-id.txt", "original_declaration_hash_cases.txt",
        "original_authored_id_search_cases.txt",
        "probe_original_light_update.py", "original_light_update_cases.txt",
        "probe_original_light_presets.py", "original_light_preset_cases.txt",
        "original_light_preset_cases.constructor.txt", "original_light_preset_cases.lerp.txt",
        "original_light_preset_cases.reflectors.txt",
        "original_light_preset_cases.distances.txt", "original_light_preset_cases.offsets.txt",
        "probe_original_light_proximity.py", "original_light_proximity_cases.txt",
        "probe_original_light_aabb.py", "native_light_aabb_cases.txt", "native_query_corner_cases.txt",
        "probe_original_viewport.py", "original_viewport_scale_cases.txt", "original_floor_cases.txt", "capture_original_light_runtime.py",
        "probe_original_light_sphere.py", "original_light_triangle_cases.txt",
        "original_light_portal_cases.txt", "original_light_sphere_query_cases.txt",
        "audit_light_rgb_transfer.py",
        "audit_light_presets.py")]
    # Executable native probes and numerical fixtures, not research notes.
    files += sorted((PROJECT / "analysis").glob("probe_original_*.py"))
    files += sorted((PROJECT / "analysis").glob("original_*_cases*.txt"))
    files += sorted(PROJECT.glob("*.ps1"))
    files += sorted((PROJECT / "src").rglob("*.rs"))
    files += sorted((PROJECT / "assets").rglob("*.wgsl"))
    files += [
        PROJECT / "vendor" / "bevy_light-0.19.1" / "src" / "point_light.rs",
        PROJECT / "vendor" / "bevy_pbr-0.19.1" / "src" / "render" / "light.rs",
        PROJECT / "vendor" / "bevy_pbr-0.19.1" / "src" / "render" / "pbr_lighting.wgsl",
        PROJECT / "vendor" / "bevy_pbr-0.19.1" / "src" / "render" / "shadows.wgsl",
        PROJECT / "vendor" / "bevy_pbr-0.19.1" / "src" / "volumetric_fog" / "volumetric_fog.wgsl",
    ]
    files += sorted(p for p in (PROJECT / "knowledge").iterdir() if p.suffix in TEXT_SUFFIXES and p.suffix != '.md' and p != SEED)
    return [(p.relative_to(PROJECT).as_posix(), p, "project")
            for p in dict.fromkeys(files) if p.is_file()]


def refresh(db: sqlite3.Connection) -> tuple[int, int]:
    changed = 0
    for label, path, origin in project_files():
        changed += index_text(db, label, path, origin)
    # SQLite is authoritative. Do not read/import the recovery snapshot or
    # resurrect retired findings during ordinary indexing.
    db.execute("DELETE FROM documents WHERE path='knowledge/seed.json'")
    db.execute("DELETE FROM documents_fts WHERE path='knowledge/seed.json'")
    assets = 0
    artifact_dir = PROJECT / "artifacts"
    if artifact_dir.is_dir():
        for path in sorted(artifact_dir.iterdir()):
            if not path.is_file():
                continue
            label = path.relative_to(PROJECT).as_posix()
            origin = "extracted" if path.suffix in GAME_SUFFIXES or ".bimage7" in path.name else "project"
            assets += 1
            changed += upsert_resource(db, label, path, "asset", origin)
        # Index both F8 report folders, without recursively indexing tool caches.
        for capture_name in ("collision-reports", "streaming-reports"):
            capture_dir = artifact_dir / capture_name
            if capture_dir.is_dir():
                for path in sorted(capture_dir.iterdir()):
                    if not path.is_file():
                        continue
                    label = path.relative_to(PROJECT).as_posix()
                    changed += upsert_resource(db, label, path, "asset", "project")
                    assets += 1
    db.commit()
    return changed, assets


def index_tree(db: sqlite3.Connection, root: Path, label: str) -> int:
    root = root.resolve()
    if not root.is_dir():
        raise ValueError(f"not a directory: {root}")
    if not label or "/" in label or "\\" in label:
        raise ValueError("label must be one simple name, e.g. ghidra-dump")
    count = 0
    for path in sorted(root.rglob("*")):
        if path.is_file() and path.suffix.lower() in TEXT_SUFFIXES:
            count += index_text(db, f"external/{label}/{path.relative_to(root).as_posix()}", path, "external")
    manifest = root / "functions.tsv"
    if manifest.is_file():
        rows = []
        for line in manifest.read_text(encoding="utf-8").splitlines()[1:]:
            fields = line.split("\t")
            if len(fields) != 6:
                raise ValueError(f"Malformed decompilation manifest row: {line[:120]}")
            address, name, namespace, body_bytes, status, chunk = fields
            chunk_path = f"external/{label}/{chunk}"
            if not db.execute("SELECT 1 FROM resources WHERE path=?", (chunk_path,)).fetchone():
                raise ValueError(f"Manifest points to unindexed chunk: {chunk}")
            rows.append((label, address, name, namespace, int(body_bytes), status, chunk_path))
        db.executemany(
            """INSERT INTO decompiled_functions(corpus,address,name,namespace,body_bytes,status,chunk_path)
               VALUES(?,?,?,?,?,?,?) ON CONFLICT(corpus,address) DO UPDATE SET
               name=excluded.name,namespace=excluded.namespace,body_bytes=excluded.body_bytes,
               status=excluded.status,chunk_path=excluded.chunk_path""", rows,
        )
        print(f"Function manifest: {len(rows)} entries; {sum(row[5] == 'ok' for row in rows)} decompiled.")
    calls_manifest = root / "calls.tsv"
    if calls_manifest.is_file():
        calls = []
        for line in calls_manifest.read_text(encoding="utf-8").splitlines()[1:]:
            fields = line.split("\t")
            if len(fields) != 2:
                raise ValueError(f"Malformed call-graph row: {line[:120]}")
            calls.append((label, fields[0], fields[1]))
        db.executemany(
            "INSERT OR IGNORE INTO function_calls(corpus,caller,callee) VALUES(?,?,?)", calls,
        )
        print(f"Call graph: {len(calls)} edges in manifest.")
    types_manifest = root / "types.tsv"
    if types_manifest.is_file():
        types = []
        for line in types_manifest.read_text(encoding="utf-8").splitlines()[1:]:
            fields = line.split("\t")
            if len(fields) != 4:
                raise ValueError(f"Malformed data-type row: {line[:120]}")
            types.append((label, fields[0], fields[1], fields[2], int(fields[3])))
        db.executemany(
            """INSERT INTO decompiled_types(corpus,category,name,kind,byte_size)
               VALUES(?,?,?,?,?) ON CONFLICT(corpus,category,name) DO UPDATE SET
               kind=excluded.kind,byte_size=excluded.byte_size""", types,
        )
        print(f"Data types: {len(types)} entries in manifest.")
    fields_manifest = root / "type_fields.tsv"
    if fields_manifest.is_file():
        fields = []
        for line in fields_manifest.read_text(encoding="utf-8").splitlines()[1:]:
            parts = line.split("\t")
            if len(parts) != 7:
                raise ValueError(f"Malformed type-field row: {line[:120]}")
            fields.append((label, parts[0], parts[1], int(parts[2]), int(parts[3]),
                           int(parts[4]), parts[5], parts[6]))
        db.executemany(
            """INSERT INTO decompiled_type_fields
               (corpus,category,parent,ordinal,byte_offset,byte_size,name,field_type)
               VALUES(?,?,?,?,?,?,?,?) ON CONFLICT(corpus,category,parent,ordinal) DO UPDATE SET
               byte_offset=excluded.byte_offset,byte_size=excluded.byte_size,
               name=excluded.name,field_type=excluded.field_type""", fields,
        )
        print(f"Composite fields: {len(fields)} entries in manifest.")
    db.commit()
    return count


def search(db: sqlite3.Connection, term: str) -> None:
    pattern = f"%{term.lower()}%"
    facts = db.execute(
        """SELECT id,subject,predicate,value,status FROM findings
           WHERE lower(id || ' ' || subject || ' ' || predicate || ' ' || value || ' ' || limitation) LIKE ?
           ORDER BY subject,id LIMIT 30""", (pattern,)).fetchall()
    symbols = db.execute(
        "SELECT kind,name,path,line FROM symbols WHERE lower(name) LIKE ? ORDER BY name LIMIT 30",
        (pattern,)).fetchall()
    phrase = '"' + term.replace('"', '""') + '"'
    docs = db.execute(
        "SELECT path FROM documents_fts WHERE documents_fts MATCH ? ORDER BY rank LIMIT 30",
        (phrase,)).fetchall()
    decompiled = db.execute(
        """SELECT address,name,namespace,status,chunk_path FROM decompiled_functions
           WHERE lower(address || ' ' || name || ' ' || namespace) LIKE ?
           ORDER BY name LIMIT 30""", (pattern,)).fetchall()
    types = db.execute(
        """SELECT category,name,kind,byte_size FROM decompiled_types
           WHERE lower(category || ' ' || name) LIKE ? ORDER BY name LIMIT 30""",
        (pattern,)).fetchall()
    print(f"Findings ({len(facts)} shown):")
    for fid, subject, predicate, value, status in facts:
        print(f"  {fid} [{status}] {subject} — {predicate}: {value}")
    print(f"Symbols ({len(symbols)} shown):")
    for kind, name, path, line in symbols:
        print(f"  {kind} {name}  {path}:{line}")
    print(f"Decompiled functions ({len(decompiled)} shown):")
    for address, name, namespace, status, chunk in decompiled:
        print(f"  {address} {namespace}::{name} [{status}] {chunk}")
    print(f"Decompiled data types ({len(types)} shown):")
    for category, name, kind, size in types:
        print(f"  {kind} {category}/{name} ({size} bytes)")
    print(f"Documents containing term ({len(docs)} shown):")
    for (path,) in docs:
        print(f"  {path}")


def show(db: sqlite3.Connection, fid: str) -> None:
    item = db.execute(
        "SELECT subject,predicate,value,status,scope,limitation FROM findings WHERE id=?", (fid,)
    ).fetchone()
    if item is None:
        raise ValueError(f"no finding: {fid}")
    for field, value in zip(("Subject", "Predicate", "Value", "Status", "Scope", "Limit"), item):
        print(f"{field}: {value}")
    print("Evidence:")
    for path, locator, method in db.execute(
        "SELECT resource_path,locator,method FROM evidence WHERE finding_id=? ORDER BY resource_path", (fid,)
    ):
        print(f"  {path} | {locator} | {method}")
        if db.execute('SELECT 1 FROM stored_evidence WHERE resource_path=?', (path,)).fetchone():
            print(f'    Stored in SQLite: python knowledge/db.py evidence {path}')


def record(db: sqlite3.Connection, payload: str) -> None:
    items = json.loads(payload)
    if isinstance(items, dict):
        items = [items]
    if not isinstance(items, list) or not items:
        raise ValueError("payload must contain one finding or a nonempty list")
    with db:
        for item in items:
            sources = item["evidence"]
            if not sources:
                raise ValueError(f"finding {item['id']} needs evidence")
            for source in sources:
                if not db.execute("SELECT 1 FROM resources WHERE path=?", (source["path"],)).fetchone():
                    raise ValueError(f"evidence resource not indexed: {source['path']}")
            db.execute(
                """INSERT INTO findings(id,subject,predicate,value,status,scope,limitation)
                   VALUES(?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET
                   subject=excluded.subject,predicate=excluded.predicate,value=excluded.value,
                   status=excluded.status,scope=excluded.scope,limitation=excluded.limitation,
                   updated_at=CURRENT_TIMESTAMP""",
                (item["id"], item["subject"], item["predicate"], item["value"],
                 item["status"], item["scope"], item.get("limitation", "")),
            )
            db.execute("DELETE FROM evidence WHERE finding_id=?", (item["id"],))
            db.executemany(
                "INSERT INTO evidence(finding_id,resource_path,locator,method) VALUES(?,?,?,?)",
                ((item["id"], source["path"], source.get("locator", ""), source["method"])
                 for source in sources),
            )
    print(f"Recorded {len(items)} finding(s).")


def finding_records(db: sqlite3.Connection) -> list[dict]:
    items = []
    for row in db.execute(
        "SELECT id,subject,predicate,value,status,scope,limitation FROM findings ORDER BY id"
    ):
        item = dict(zip(("id", "subject", "predicate", "value", "status", "scope", "limitation"), row))
        item["evidence"] = [
            {"path": path, "locator": locator, "method": method}
            for path, locator, method in db.execute(
                "SELECT resource_path,locator,method FROM evidence WHERE finding_id=? ORDER BY resource_path,locator",
                (item["id"],),
            )
        ]
        items.append(item)
    return items


def export_findings(db: sqlite3.Connection, output: Path) -> None:
    items = finding_records(db)
    output.write_text(json.dumps(items, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"Exported {len(items)} findings to {output}.")


def audit(db: sqlite3.Connection) -> bool:
    missing = db.execute(
        """SELECT e.finding_id,e.resource_path FROM evidence e
           LEFT JOIN resources r ON r.path=e.resource_path WHERE r.path IS NULL"""
    ).fetchall()
    unreferenced = db.execute(
        """SELECT f.id FROM findings f LEFT JOIN evidence e ON e.finding_id=f.id
           WHERE e.finding_id IS NULL"""
    ).fetchall()
    print(f"Resources: {db.execute('SELECT count(*) FROM resources').fetchone()[0]}")
    print(f"Indexed text documents: {db.execute('SELECT count(*) FROM documents').fetchone()[0]}")
    print(f"Rust symbols (text scan): {db.execute('SELECT count(*) FROM symbols').fetchone()[0]}")
    print(f"Decompiled functions: {db.execute('SELECT count(*) FROM decompiled_functions').fetchone()[0]}")
    print(f"Direct call edges: {db.execute('SELECT count(*) FROM function_calls').fetchone()[0]}")
    print(f"Decompiled data types: {db.execute('SELECT count(*) FROM decompiled_types').fetchone()[0]}")
    print(f"Composite fields: {db.execute('SELECT count(*) FROM decompiled_type_fields').fetchone()[0]}")
    print(f"Findings: {db.execute('SELECT count(*) FROM findings').fetchone()[0]}")
    print(f"Evidence links: {db.execute('SELECT count(*) FROM evidence').fetchone()[0]}")
    print(f"Broken evidence references: {len(missing)}; findings without evidence: {len(unreferenced)}")
    for fid, path in missing:
        print(f"  broken: {fid} -> {path}")
    for (fid,) in unreferenced:
        print(f"  no evidence: {fid}")
    return not missing and not unreferenced


def main() -> None:
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--db", type=Path, default=DEFAULT_DB, help="SQLite database file")
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("refresh", help="Index project files and local asset metadata without importing snapshots")
    commands.add_parser("audit", help="Count records and check evidence links")
    query = commands.add_parser("search", help="Search findings, symbols, and source text")
    query.add_argument("term")
    detail = commands.add_parser("show", help="Show one finding and its evidence")
    detail.add_argument("id")
    update = commands.add_parser("record", help="Upsert a finding and replace its evidence")
    update.add_argument("--payload", required=True, help="JSON object or array")
    retire = commands.add_parser("retire", help="Remove one obsolete finding and its evidence")
    retire.add_argument("id")
    snapshot = commands.add_parser("export-findings", help="Export DB findings as a portable bootstrap snapshot")
    snapshot.add_argument("output", type=Path)
    external = commands.add_parser("index-tree", help="Index a separate text/decompilation directory")
    external.add_argument("directory", type=Path)
    external.add_argument("--label", required=True)
    function = commands.add_parser("function", help="Original address -> Rust links, evidence, scope and blocker")
    function.add_argument("reference")
    commands.add_parser("coverage", help="Report function rewrite coverage and remaining work")
    commands.add_parser("backfill-functions", help="Link existing finding mentions without assuming full coverage")
    for command in ("record-function", "inspect-function", "read-evidence"):
        update_function = commands.add_parser(command)
        update_function.add_argument("--payload-file", type=Path, required=True, help="JSON file or '-' for stdin")
    stored = commands.add_parser('evidence', help='Read archived evidence or indexed text directly from SQLite')
    stored.add_argument('path')
    stored.add_argument('--start-line', type=int, default=1)
    stored.add_argument('--max-lines', type=int, default=80)
    args = parser.parse_args()
    with connect(args.db) as db:
        if args.command == "refresh":
            changed, assets = refresh(db)
            print(f"Refreshed {changed} changed resources; {assets} local artifacts inventoried.")
            audit(db)
        elif args.command == "audit":
            if not audit(db):
                raise SystemExit(1)
        elif args.command == "search":
            search(db, args.term)
        elif args.command == "show":
            show(db, args.id)
        elif args.command == "record":
            record(db, args.payload)
        elif args.command == "retire":
            with db:
                count = db.execute("DELETE FROM findings WHERE id=?", (args.id,)).rowcount
            if count != 1:
                raise ValueError(f"no finding: {args.id}")
            print(f"Retired {args.id}.")
        elif args.command == "export-findings":
            export_findings(db, args.output)
        elif args.command == "index-tree":
            print(f"Indexed {index_tree(db, args.directory, args.label)} changed text files.")
        elif args.command == "function":
            print(json.dumps(functions.lookup(db, args.reference), ensure_ascii=False, indent=2))
        elif args.command == "coverage":
            print(json.dumps(functions.coverage(db), ensure_ascii=False, indent=2))
        elif args.command == "backfill-functions":
            print(json.dumps(functions.backfill(db), ensure_ascii=False, indent=2))
        elif args.command in ("record-function", "inspect-function", "read-evidence"):
            if args.payload_file.resolve() == SEED.resolve():
                raise ValueError('Do not read seed.json')
            payload = json.loads(sys.stdin.read() if str(args.payload_file) == '-' else args.payload_file.read_text(encoding="utf-8-sig"))
            handler = {'record-function': functions.record_function, 'inspect-function': functions.record_inspection,
                       'read-evidence': read_evidence}[args.command]
            print(json.dumps(handler(db, payload), ensure_ascii=False))
        elif args.command == 'evidence':
            print(json.dumps(read_evidence(db, vars(args)), ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
