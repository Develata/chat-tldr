CREATE TABLE semantic_relations (
  relation_id TEXT PRIMARY KEY,
  chat_id TEXT NOT NULL REFERENCES chats,
  topic_id TEXT NOT NULL REFERENCES topics,
  source_id TEXT NOT NULL REFERENCES messages(message_id),
  target_id TEXT REFERENCES messages(message_id),
  body_json TEXT NOT NULL
);
CREATE INDEX idx_relations_chat ON semantic_relations(chat_id, source_id, target_id);
CREATE INDEX idx_relations_topic ON semantic_relations(topic_id, relation_id);
CREATE TABLE relation_coverage (
  message_id TEXT PRIMARY KEY REFERENCES messages(message_id),
  run_id TEXT NOT NULL REFERENCES runs
);
