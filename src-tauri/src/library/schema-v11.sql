ALTER TABLE research_topics ADD COLUMN ai_source_json TEXT;
ALTER TABLE library_ideas ADD COLUMN ai_source_json TEXT;
UPDATE library_schema SET version=11 WHERE id=1;
