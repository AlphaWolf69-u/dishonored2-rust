PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS resources (
    path TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    origin TEXT NOT NULL CHECK (origin IN ('project', 'extracted', 'external')),
    byte_size INTEGER,
    sha256 TEXT,
    indexed_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS documents (
    path TEXT PRIMARY KEY REFERENCES resources(path) ON DELETE CASCADE,
    content TEXT NOT NULL
);

CREATE VIRTUAL TABLE IF NOT EXISTS documents_fts USING fts5(
    path UNINDEXED, content, tokenize='unicode61'
);

CREATE TABLE IF NOT EXISTS symbols (
    path TEXT NOT NULL REFERENCES resources(path) ON DELETE CASCADE,
    line INTEGER NOT NULL,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    PRIMARY KEY (path, line, kind, name)
);
CREATE INDEX IF NOT EXISTS symbols_name ON symbols(name);

CREATE TABLE IF NOT EXISTS decompiled_functions (
    corpus TEXT NOT NULL,
    address TEXT NOT NULL,
    name TEXT NOT NULL,
    namespace TEXT NOT NULL,
    body_bytes INTEGER NOT NULL,
    status TEXT NOT NULL,
    chunk_path TEXT NOT NULL REFERENCES resources(path),
    PRIMARY KEY (corpus, address)
);
CREATE INDEX IF NOT EXISTS decompiled_functions_name ON decompiled_functions(name);
CREATE INDEX IF NOT EXISTS decompiled_functions_chunk ON decompiled_functions(chunk_path);

CREATE TABLE IF NOT EXISTS function_calls (
    corpus TEXT NOT NULL,
    caller TEXT NOT NULL,
    callee TEXT NOT NULL,
    PRIMARY KEY (corpus, caller, callee)
);
CREATE INDEX IF NOT EXISTS function_calls_callee ON function_calls(corpus, callee);

CREATE TABLE IF NOT EXISTS decompiled_types (
    corpus TEXT NOT NULL,
    category TEXT NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    byte_size INTEGER NOT NULL,
    PRIMARY KEY (corpus, category, name)
);
CREATE INDEX IF NOT EXISTS decompiled_types_name ON decompiled_types(name);

CREATE TABLE IF NOT EXISTS decompiled_type_fields (
    corpus TEXT NOT NULL,
    category TEXT NOT NULL,
    parent TEXT NOT NULL,
    ordinal INTEGER NOT NULL,
    byte_offset INTEGER NOT NULL,
    byte_size INTEGER NOT NULL,
    name TEXT NOT NULL,
    field_type TEXT NOT NULL,
    PRIMARY KEY (corpus, category, parent, ordinal)
);
CREATE INDEX IF NOT EXISTS decompiled_type_fields_name ON decompiled_type_fields(name);

CREATE TABLE IF NOT EXISTS findings (
    id TEXT PRIMARY KEY,
    subject TEXT NOT NULL,
    predicate TEXT NOT NULL,
    value TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('observed', 'implemented', 'inferred', 'open')),
    scope TEXT NOT NULL,
    limitation TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS findings_subject ON findings(subject);

CREATE TABLE IF NOT EXISTS evidence (
    finding_id TEXT NOT NULL REFERENCES findings(id) ON DELETE CASCADE,
    resource_path TEXT NOT NULL REFERENCES resources(path),
    locator TEXT NOT NULL DEFAULT '',
    method TEXT NOT NULL,
    PRIMARY KEY (finding_id, resource_path, locator)
);

-- Function coverage is independent of a finding's narrower implementation scope.
CREATE TABLE IF NOT EXISTS function_register (
    corpus TEXT NOT NULL,
    address TEXT NOT NULL,
    name TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'referenced' CHECK (status IN
        ('referenced','inspected','understood','partial','implemented','verified','mapped','blocked')),
    summary TEXT NOT NULL DEFAULT '',
    scope TEXT NOT NULL DEFAULT '',
    blocker TEXT NOT NULL DEFAULT '',
    first_inspected_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (corpus,address),
    CHECK (status != 'blocked' OR length(trim(blocker)) > 0)
);
CREATE INDEX IF NOT EXISTS function_register_status ON function_register(status);

CREATE TABLE IF NOT EXISTS function_implementations (
    corpus TEXT NOT NULL,
    address TEXT NOT NULL,
    path TEXT NOT NULL,
    symbol TEXT NOT NULL,
    kind TEXT NOT NULL DEFAULT 'rust' CHECK (kind IN ('rust','existing','stdlib','dependency')),
    relationship TEXT NOT NULL DEFAULT 'candidate' CHECK (relationship IN ('candidate','confirmed')),
    PRIMARY KEY (corpus,address,path,symbol),
    FOREIGN KEY (corpus,address) REFERENCES function_register(corpus,address) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS function_findings (
    corpus TEXT NOT NULL,
    address TEXT NOT NULL,
    finding_id TEXT NOT NULL REFERENCES findings(id) ON DELETE CASCADE,
    relationship TEXT NOT NULL DEFAULT 'mentions' CHECK (relationship IN ('mentions','supports')),
    PRIMARY KEY (corpus,address,finding_id),
    FOREIGN KEY (corpus,address) REFERENCES function_register(corpus,address) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS function_evidence (
    corpus TEXT NOT NULL,
    address TEXT NOT NULL,
    resource_path TEXT NOT NULL REFERENCES resources(path),
    locator TEXT NOT NULL DEFAULT '',
    method TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('original','implementation','verification')),
    PRIMARY KEY (corpus,address,resource_path,locator,kind),
    FOREIGN KEY (corpus,address) REFERENCES function_register(corpus,address) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS function_inspections (
    id TEXT PRIMARY KEY,
    corpus TEXT NOT NULL,
    address TEXT,
    reference TEXT NOT NULL,
    tool TEXT NOT NULL,
    request_json TEXT NOT NULL,
    capture_path TEXT NOT NULL REFERENCES resources(path),
    session_id TEXT NOT NULL,
    confirmed INTEGER NOT NULL DEFAULT 1 CHECK (confirmed IN (0,1)),
    inspected_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (corpus,address) REFERENCES function_register(corpus,address)
);
CREATE INDEX IF NOT EXISTS function_inspections_address ON function_inspections(corpus,address);

-- Legacy notes and retired diagnostics remain readable after file cleanup.
CREATE TABLE IF NOT EXISTS stored_evidence (
    resource_path TEXT PRIMARY KEY REFERENCES resources(path),
    payload BLOB NOT NULL,
    encoding TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    saved_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
