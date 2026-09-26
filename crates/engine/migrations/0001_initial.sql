CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);   -- db_version 等

CREATE TABLE chats (
  chat_id        TEXT PRIMARY KEY,
  kind           TEXT NOT NULL,            -- group | private
  display_name   TEXT NOT NULL,
  self_uid       TEXT, self_uin TEXT,      -- 来自导出文件或 --self-uid/--self-uin
  last_ingested  TEXT,                     -- Cursor 字符串
  last_analyzed  TEXT,
  last_reviewed  TEXT,
  updated_at     TEXT NOT NULL
);

CREATE TABLE persons (person_id TEXT PRIMARY KEY, uin TEXT, first_seen TEXT NOT NULL);
CREATE TABLE person_aliases (                 -- 昵称/群名片历史，只是可变属性
  person_id TEXT NOT NULL REFERENCES persons, chat_id TEXT, alias TEXT NOT NULL,
  alias_kind TEXT NOT NULL,                   -- name | nickname | group_card | remark
  first_seen TEXT NOT NULL, last_seen TEXT NOT NULL,
  PRIMARY KEY (person_id, chat_id, alias, alias_kind)
);

CREATE TABLE imports (
  import_id TEXT PRIMARY KEY, file_path TEXT NOT NULL, file_hash TEXT NOT NULL,
  chat_id TEXT NOT NULL, run_id TEXT NOT NULL,
  seen INTEGER NOT NULL, inserted INTEGER NOT NULL, duplicate INTEGER NOT NULL,
  backfilled INTEGER NOT NULL,                -- 插入位置早于 last_analyzed 的消息数
  imported_at TEXT NOT NULL
);

CREATE TABLE messages (
  pk               INTEGER PRIMARY KEY AUTOINCREMENT,   -- Cursor.ordinal
  message_id       TEXT NOT NULL UNIQUE,
  chat_id          TEXT NOT NULL REFERENCES chats,
  source_identity  TEXT NOT NULL,
  sender           TEXT NOT NULL,
  sender_display   TEXT NOT NULL,
  sent_at_ms       INTEGER NOT NULL,
  text             TEXT NOT NULL,
  reply_source_id  TEXT, reply_resolved TEXT,           -- 悬空引用在后续导入时补全
  recalled         INTEGER NOT NULL, system INTEGER NOT NULL,
  body_json        TEXT NOT NULL,                        -- 完整 UnifiedMessage
  analysis_state   TEXT NOT NULL DEFAULT 'pending',      -- pending | done | skipped | failed，见 PIPELINE §1.1
  analyzed_run     TEXT,                                 -- 置为 done 的那次运行
  UNIQUE (chat_id, source_identity)
);
CREATE INDEX idx_messages_order ON messages(chat_id, sent_at_ms, pk);
CREATE INDEX idx_messages_reply ON messages(chat_id, reply_source_id) WHERE reply_resolved IS NULL;
CREATE TABLE message_mentions (message_id TEXT NOT NULL, target_kind TEXT NOT NULL, uid TEXT, uin TEXT);

CREATE TABLE topics (
  topic_id TEXT PRIMARY KEY, chat_id TEXT NOT NULL,
  title TEXT NOT NULL, title_is_provisional INTEGER NOT NULL,
  state TEXT NOT NULL,                    -- active | closed | merged
  merged_into TEXT, last_message_at TEXT NOT NULL,
  is_chitchat REAL,                       -- Jev noul 概率
  created_in_run TEXT NOT NULL, updated_at TEXT NOT NULL
);
CREATE TABLE topic_messages (
  topic_id TEXT NOT NULL, message_id TEXT NOT NULL, burst_id TEXT NOT NULL,
  method TEXT NOT NULL,                   -- rule_reply | jev | llm_review | new_topic | direct | similarity
  confidence REAL, run_id TEXT NOT NULL,
  PRIMARY KEY (message_id)
);

CREATE TABLE insights (
  insight_id TEXT PRIMARY KEY, chat_id TEXT NOT NULL, topic_id TEXT,
  kind TEXT NOT NULL, priority TEXT NOT NULL, rank_prior REAL NOT NULL,
  verification_status TEXT NOT NULL, lifecycle TEXT NOT NULL,
  body_json TEXT NOT NULL,                -- 完整 Insight
  created_in_run TEXT NOT NULL, updated_at TEXT NOT NULL
);
CREATE TABLE evidence (
  insight_id TEXT NOT NULL REFERENCES insights, message_id TEXT NOT NULL,
  quote TEXT NOT NULL, render_profile TEXT NOT NULL, ok INTEGER NOT NULL
);

CREATE TABLE feedback (
  feedback_id INTEGER PRIMARY KEY, insight_id TEXT NOT NULL UNIQUE,
  label TEXT NOT NULL,                    -- useful | not_important
  created_at TEXT NOT NULL
);
CREATE TABLE preference_weights (         -- 个性化修正；见 PIPELINE §7.3
  feature TEXT PRIMARY KEY, weight REAL NOT NULL, n INTEGER NOT NULL, updated_at TEXT NOT NULL
);

CREATE TABLE model_cache (
  cache_key TEXT PRIMARY KEY,             -- BLAKE3，见 PIPELINE §9
  stage TEXT NOT NULL, provider TEXT NOT NULL, model TEXT NOT NULL,
  response_json TEXT NOT NULL, input_tokens INTEGER, output_tokens INTEGER,
  created_at TEXT NOT NULL
);

CREATE TABLE runs (
  run_id TEXT PRIMARY KEY, command TEXT NOT NULL, chat_id TEXT,
  status TEXT NOT NULL,                   -- running | partial | complete | failed | cancelled
  pid INTEGER, args_json TEXT NOT NULL,
  started_at TEXT NOT NULL, heartbeat_at TEXT NOT NULL, finished_at TEXT
);
CREATE TABLE run_checkpoints (            -- 按话题的检查点
  run_id TEXT NOT NULL, topic_id TEXT NOT NULL, stage TEXT NOT NULL,
  status TEXT NOT NULL, updated_at TEXT NOT NULL, PRIMARY KEY (run_id, topic_id, stage)
);
CREATE TABLE decisions (                  -- 智能体决策日志
  run_id TEXT NOT NULL, step INTEGER NOT NULL,
  observation_json TEXT NOT NULL, allowed_json TEXT NOT NULL,
  chosen_json TEXT NOT NULL, method TEXT NOT NULL,   -- rule | jev | fallback
  probabilities_json TEXT, reason TEXT NOT NULL, created_at TEXT NOT NULL,
  PRIMARY KEY (run_id, step)
);
CREATE TABLE jev_answers (                -- 全部 Jev 概率，供校准曲线
  run_id TEXT NOT NULL, request_key TEXT NOT NULL, question_id TEXT NOT NULL,
  qtype TEXT NOT NULL, answer_json TEXT NOT NULL, confidence REAL, subject_json TEXT NOT NULL
);
CREATE TABLE usage (
  run_id TEXT NOT NULL, stage TEXT NOT NULL, provider TEXT NOT NULL, model TEXT NOT NULL,
  calls INTEGER NOT NULL, cache_hits INTEGER NOT NULL,
  input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL, cost_usd REAL NOT NULL,
  PRIMARY KEY (run_id, stage, provider, model)
);
