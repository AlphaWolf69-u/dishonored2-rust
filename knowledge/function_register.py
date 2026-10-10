"""Address-based rewrite coverage; finding mentions never imply a complete port."""
from __future__ import annotations

import hashlib
import json
import re
import sqlite3
from pathlib import Path

PROJECT = Path(__file__).resolve().parent.parent
CORPUS = "dishonored2-exe"
ADDRESS = re.compile(r"\b(?:FUN_|0x)?(14[0-9a-fA-F]{7})\b")
STATUSES = {"referenced", "inspected", "understood", "partial", "implemented", "verified", "mapped", "blocked"}


def address_of(reference: str) -> str | None:
    value = re.sub(r"^(?:FUN_|0x)", "", reference.strip(), flags=re.I).lower()
    return value if re.fullmatch(r"[0-9a-f]{8,16}", value) else None


def resolve(db: sqlite3.Connection, reference: str, corpus: str = CORPUS):
    address = address_of(reference)
    column, value = ("address", address) if address else ("name", reference)
    row = db.execute(f"SELECT address,name FROM decompiled_functions WHERE corpus=? AND {column}=? LIMIT 1", (corpus, value)).fetchone()
    if not row:
        row = db.execute(
            f"SELECT address,name FROM function_register WHERE corpus=? AND {column}=? LIMIT 1",
            (corpus, value),
        ).fetchone()
    return row


def ensure(db, corpus, address, name=""):
    # An automatic inspection must not erase a manually verified status or blocker.
    db.execute(
        """INSERT INTO function_register(corpus,address,name) VALUES(?,?,?)
        ON CONFLICT(corpus,address) DO UPDATE SET
        name=CASE WHEN excluded.name!='' THEN excluded.name ELSE function_register.name END""",
        (corpus, address, name),
    )


def resource(db, label: str):
    if label.replace("\\", "/").lower().endswith("knowledge/seed.json"):
        raise ValueError("seed.json is not an evidence source")
    if db.execute("SELECT 1 FROM resources WHERE path=?", (label,)).fetchone():
        return
    file = (PROJECT / label).resolve()
    if not file.is_relative_to(PROJECT.resolve()) or not file.is_file():
        raise ValueError(f"Evidence must be indexed or a project file: {label}")
    data = file.read_bytes()
    db.execute(
        "INSERT INTO resources(path,kind,origin,byte_size,sha256) VALUES(?,?,?,?,?)",
        (label, "note", "project", len(data), hashlib.sha256(data).hexdigest()),
    )


def add_evidence(db, corpus, address, item):
    resource(db, item["path"])
    if not item.get("method", "").strip():
        raise ValueError("Evidence requires a method")
    db.execute(
        """INSERT INTO function_evidence(corpus,address,resource_path,locator,method,kind)
        VALUES(?,?,?,?,?,?) ON CONFLICT(corpus,address,resource_path,locator,kind)
        DO UPDATE SET method=excluded.method""",
        (corpus, address, item["path"], item.get("locator", ""), item["method"], item["kind"]),
    )


def record_inspection(db, item):
    corpus = item.get("corpus", CORPUS)
    reference = str(item["reference"])
    entry = address_of(str(item.get("entry_address", "")))
    found = resolve(db, entry or reference, corpus)
    if not found and entry:
        found = (entry, item.get("entry_name", "FUN_" + entry))
    # Interior instruction addresses are not guessed into the nearest function.
    address, name = found if found else (None, "")
    request = json.dumps(item.get("request", {}), sort_keys=True)
    capture = item["capture_path"]
    session = item.get("session_id", "unknown")
    confirmed = int(bool(item.get("confirmed", True)))
    key = item.get("id") or hashlib.sha256(
        json.dumps([corpus, reference, item["tool"], request, capture, session,
                    item.get("tool_call_id", "")]).encode()
    ).hexdigest()
    with db:
        resource(db, capture)
        if address:
            ensure(db, corpus, address, name)
            if confirmed:
                db.execute("UPDATE function_register SET status='inspected' WHERE corpus=? AND address=? AND status='referenced'", (corpus, address))
                add_evidence(db, corpus, address, {
                "path": capture, "locator": f"request {reference}", "kind": "original",
                "method": item.get("method", "Saved Ghidra response and request; no coverage inferred"),
                })
        db.execute(
            """INSERT OR IGNORE INTO function_inspections
            (id,corpus,address,reference,tool,request_json,capture_path,session_id,confirmed)
            VALUES(?,?,?,?,?,?,?,?,?)""",
            (key, corpus, address, reference, item["tool"], request, capture, session, confirmed),
        )
    return {"address": address, "reference": reference, "capture_path": capture}


def record_function(db, item):
    corpus = item.get("corpus", CORPUS)
    reference = str(item["address"])
    found = resolve(db, reference, corpus)
    address, name = found if found else (address_of(reference), item.get("name", ""))
    if not address:
        raise ValueError("Expected original function entry address or indexed function name")
    name = name or item.get("name", "FUN_" + address)
    status = item["status"]
    if status not in STATUSES:
        raise ValueError(f"Unknown status: {status}")
    scope, blocker = item.get("scope", "").strip(), item.get("blocker", "").strip()
    summary = item.get("summary", "").strip()
    evidence, implementations = item.get("evidence", []), item.get("implementations", [])
    kinds = {source["kind"] for source in evidence}
    if status in {"understood", "partial", "implemented", "verified", "mapped"}:
        if not scope or not summary or "original" not in kinds:
            raise ValueError("Coverage claims require a summary, scope, and original evidence")
    if status in {"partial", "implemented", "verified", "mapped"} and not implementations:
        raise ValueError("Implementation status requires explicit implementation links")
    if status in {"partial", "blocked"} and not blocker:
        raise ValueError("Partial/blocked functions require the exact remaining work or blocker")
    if status in {"verified", "mapped"} and ("verification" not in kinds or blocker):
        raise ValueError("Verified/mapped requires verification evidence and no blocker")
    with db:
        ensure(db, corpus, address, name)
        for impl in implementations:
            kind = impl.get("kind", "rust")
            if kind in {"rust", "existing"}:
                source = (PROJECT / impl["path"]).resolve()
                if not source.is_relative_to(PROJECT.resolve()) or not source.is_file():
                    raise ValueError(f"Missing Rust source: {impl['path']}")
                if not re.search(r"\bfn\s+" + re.escape(impl["symbol"]) + r"\b", source.read_text(encoding="utf-8")):
                    raise ValueError(f"Missing Rust function: {impl['symbol']}")
            elif kind not in {"stdlib", "dependency"}:
                raise ValueError(f"Unknown implementation kind: {kind}")
            db.execute(
                """INSERT INTO function_implementations(corpus,address,path,symbol,kind,relationship)
                VALUES(?,?,?,?,?,'confirmed') ON CONFLICT(corpus,address,path,symbol)
                DO UPDATE SET kind=excluded.kind,relationship='confirmed'""",
                (corpus, address, impl["path"], impl["symbol"], kind),
            )
        for source in evidence:
            add_evidence(db, corpus, address, source)
        for fid in item.get("findings", []):
            db.execute(
                """INSERT INTO function_findings(corpus,address,finding_id,relationship)
                VALUES(?,?,?,'supports') ON CONFLICT(corpus,address,finding_id)
                DO UPDATE SET relationship='supports'""", (corpus, address, fid),
            )
        db.execute(
            """UPDATE function_register SET status=?,summary=?,scope=?,blocker=?,updated_at=CURRENT_TIMESTAMP
            WHERE corpus=? AND address=?""", (status, summary, scope, blocker, corpus, address),
        )
    return lookup(db, address, corpus)


def rows(db, sql, args):
    cursor = db.execute(sql, args)
    return [dict(zip((column[0] for column in cursor.description), row)) for row in cursor]


def describe_evidence(db, items):
    for item in items:
        path = item.get('path') or item.get('capture_path') or item.get('chunk_path')
        if path and db.execute('SELECT 1 FROM documents WHERE path=?', (path,)).fetchone():
            item.update(storage='sqlite', reader='dh2_evidence_read', read_command=f'python knowledge/db.py evidence {path}')


def lookup(db, reference, corpus=CORPUS):
    found = resolve(db, reference, corpus)
    if not found:
        return {"corpus": corpus, "reference": reference, "status": "unresolved",
                "note": "Resolve the original function entry; no nearest-address guess."}
    address, name = found
    args = (corpus, address)
    state = rows(db, "SELECT * FROM function_register WHERE corpus=? AND address=?", args)
    result = state[0] if state else {"corpus": corpus, "address": address, "name": name, "status": "not_inspected"}
    result["implementations"] = rows(db, "SELECT path,symbol,kind,relationship FROM function_implementations WHERE corpus=? AND address=?", args)
    result["findings"] = rows(db, """SELECT f.id,f.subject,f.predicate,f.value,f.status,f.scope,f.limitation,l.relationship
        FROM function_findings l JOIN findings f ON f.id=l.finding_id WHERE l.corpus=? AND l.address=?
        ORDER BY CASE l.relationship WHEN 'supports' THEN 0 ELSE 1 END,f.updated_at DESC""", args)
    for finding in result["findings"]:
        value = finding.pop("value")
        finding["summary"] = value[:800]
        finding["summary_truncated"] = len(value) > 800
        finding["details_command"] = f"python knowledge/db.py show {finding['id']}"
    result["evidence"] = rows(db, """SELECT resource_path AS path,locator,method,kind FROM function_evidence
        WHERE corpus=? AND address=? ORDER BY kind DESC,resource_path LIMIT 16""", args)
    describe_evidence(db, result['evidence'])
    result["recent_inspections"] = rows(db, """SELECT reference,tool,capture_path,confirmed,inspected_at FROM function_inspections
        WHERE corpus=? AND address=? ORDER BY inspected_at DESC LIMIT 5""", args)
    describe_evidence(db, result['recent_inspections'])
    result["cached_decompilation"] = rows(db, "SELECT chunk_path,name FROM decompiled_functions WHERE corpus=? AND address=?", args)
    describe_evidence(db, result['cached_decompilation'])
    return result


def backfill(db):
    links = 0
    with db:
        for fid, subject, predicate, value, scope, limitation in db.execute(
                "SELECT id,subject,predicate,value,scope,limitation FROM findings").fetchall():
            sources = db.execute("SELECT resource_path,locator FROM evidence WHERE finding_id=?", (fid,)).fetchall()
            text = " ".join([subject, predicate, value, scope, limitation] + [locator for _, locator in sources])
            for address in {a.lower() for a in ADDRESS.findall(text)}:
                found = resolve(db, address)
                if not found:
                    continue
                ensure(db, CORPUS, *found)
                links += db.execute(
                    "INSERT OR IGNORE INTO function_findings(corpus,address,finding_id) VALUES(?,?,?)",
                    (CORPUS, address, fid),
                ).rowcount
                # A finding may cover one branch or mention an unported callee.
                # Never promote its `implemented` status into function coverage.
                for source, locator in sources:
                    if not source.endswith(".rs"):
                        continue
                    for (symbol,) in db.execute("SELECT name FROM symbols WHERE path=? AND kind='fn'", (source,)):
                        if re.search(r"\b" + re.escape(symbol) + r"\b", locator):
                            db.execute("""INSERT OR IGNORE INTO function_implementations
                                (corpus,address,path,symbol) VALUES(?,?,?,?)""", (CORPUS, address, source, symbol))
    return {"new_finding_links": links, "automatic_verified_promotions": 0, **coverage(db)}


def coverage(db):
    return {
        "original_functions_in_corpus": db.execute("SELECT count(*) FROM decompiled_functions WHERE corpus=?", (CORPUS,)).fetchone()[0],
        "registered": db.execute("SELECT count(*) FROM function_register WHERE corpus=?", (CORPUS,)).fetchone()[0],
        "statuses": dict(db.execute("SELECT status,count(*) FROM function_register WHERE corpus=? GROUP BY status", (CORPUS,))),
        "inspections": db.execute("SELECT count(*) FROM function_inspections").fetchone()[0],
        "confirmed_inspections": db.execute("SELECT count(*) FROM function_inspections WHERE confirmed=1").fetchone()[0],
        "unresolved_inspections": db.execute("SELECT count(*) FROM function_inspections WHERE address IS NULL").fetchone()[0],
        "remaining_sample": rows(db, """SELECT address,name,status,summary,blocker FROM function_register
            WHERE corpus=? AND status NOT IN ('verified','mapped') ORDER BY updated_at DESC,address LIMIT 20""", (CORPUS,)),
    }
