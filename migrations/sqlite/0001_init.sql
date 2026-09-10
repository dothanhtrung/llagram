CREATE TABLE IF NOT EXISTS thread
(
    id         INTEGER                                             NOT NULL
        CONSTRAINT thread_pk
            PRIMARY KEY AUTOINCREMENT,
    user_id    INTEGER                                             NOT NULL,
    title      TEXT    DEFAULT ''                                  NOT NULL,
    summary    TEXT    DEFAULT ''                                  NOT NULL,
    model      TEXT    DEFAULT ''                                  NOT NULL,
    created_at TEXT    DEFAULT (STRFTIME('%Y-%m-%d %H:%M:%f', 'NOW')) NOT NULL,
    updated_at TEXT    DEFAULT (STRFTIME('%Y-%m-%d %H:%M:%f', 'NOW')) NOT NULL
);

CREATE INDEX IF NOT EXISTS thread_user_updated
    ON thread (user_id, updated_at DESC);

CREATE TABLE IF NOT EXISTS user_state
(
    user_id           INTEGER NOT NULL
        CONSTRAINT user_state_pk PRIMARY KEY,
    current_thread_id INTEGER
        REFERENCES thread (id)
            ON DELETE SET NULL
);

CREATE TABLE IF NOT EXISTS user_skill
(
    user_id INTEGER NOT NULL,
    skill   TEXT    NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 0,
    CONSTRAINT user_skill_pk PRIMARY KEY (user_id, skill)
);
