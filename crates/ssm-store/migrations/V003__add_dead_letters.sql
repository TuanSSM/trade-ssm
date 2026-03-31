CREATE TABLE IF NOT EXISTS dead_letters (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp    INTEGER NOT NULL,
    topic        TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    error        TEXT NOT NULL,
    retry_count  INTEGER NOT NULL DEFAULT 0,
    max_retries  INTEGER NOT NULL DEFAULT 3,
    next_retry_at INTEGER,
    resolved     INTEGER NOT NULL DEFAULT 0,
    created_at   INTEGER NOT NULL DEFAULT (strftime('%s', 'now') * 1000)
);

CREATE INDEX IF NOT EXISTS idx_dead_letters_next_retry ON dead_letters(next_retry_at) WHERE resolved = 0;
