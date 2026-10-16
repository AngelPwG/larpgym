CREATE TABLE metadata(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE records(id TEXT PRIMARY KEY, json TEXT NOT NULL CHECK(json_valid(json)));
CREATE TABLE remote(id TEXT PRIMARY KEY, json TEXT NOT NULL CHECK(json_valid(json)));
CREATE TABLE outbox(
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    entity_id TEXT NOT NULL,
    json TEXT NOT NULL CHECK(json_valid(json)),
    after_id TEXT,
    frozen INTEGER NOT NULL DEFAULT 0 CHECK(frozen IN(0,1))
);
CREATE INDEX outbox_entity ON outbox(entity_id,seq);
CREATE TABLE conflicts(entity_id TEXT PRIMARY KEY, reason TEXT NOT NULL);
CREATE TABLE resolutions(seq INTEGER PRIMARY KEY, json TEXT NOT NULL, resolved_at TEXT NOT NULL, choice TEXT NOT NULL);
CREATE TABLE operations(id TEXT PRIMARY KEY, request TEXT NOT NULL, outcome TEXT NOT NULL);
CREATE TABLE changes(seq INTEGER PRIMARY KEY AUTOINCREMENT, json TEXT NOT NULL CHECK(json_valid(json)));
