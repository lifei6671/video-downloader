WITH selected_categories(id) AS (
    SELECT value FROM json_each(?2)
), hits AS (
    SELECT id AS asset_id, 'title' AS field, title AS body, NULL AS document_id,
        NULL AS version, NULL AS note_id, 0 AS priority
    FROM library_assets WHERE ?1!='' AND instr(lower(title),lower(?1))>0
    UNION ALL
    SELECT id,'author',author,NULL,NULL,NULL,1 FROM library_assets
        WHERE ?1!='' AND instr(lower(author),lower(?1))>0
    UNION ALL
    SELECT asset_id,kind,text,id,version,NULL,CASE kind WHEN 'edited' THEN 2 ELSE 3 END
    FROM asset_documents WHERE ?1!='' AND instr(lower(text),lower(?1))>0
    UNION ALL
    SELECT asset_id,'note',body,NULL,NULL,id,4 FROM asset_notes
        WHERE ?1!='' AND instr(lower(body),lower(?1))>0
            AND id NOT IN (SELECT value FROM json_each(?10))
), ranked AS (
    SELECT asset_id,field,substr(body,max(1,instr(lower(body),lower(?1))-40),120) AS excerpt,document_id,version,note_id,
        row_number() OVER (PARTITION BY asset_id ORDER BY priority,version DESC,document_id,note_id) AS rank
    FROM hits
)
SELECT a.id,a.title,a.author,a.platform,a.video_id,a.canonical_url,a.created_at,a.updated_at,a.archived_at,
    r.field,r.excerpt,r.document_id,r.version,r.note_id,
    (SELECT json_group_array(category_id) FROM asset_categories WHERE asset_id=a.id),
    (SELECT json_group_array(tag_id) FROM asset_tags WHERE asset_id=a.id),
    EXISTS(SELECT 1 FROM asset_documents WHERE asset_id=a.id)
FROM library_assets a LEFT JOIN ranked r ON r.asset_id=a.id AND r.rank=1
WHERE (?1='' OR r.asset_id IS NOT NULL)
    AND (a.archived_at IS NOT NULL)=?4
    AND (?5 IS NULL OR coalesce(a.platform,'local')=?5)
    AND (json_array_length(?2)=0 OR EXISTS(SELECT 1 FROM asset_categories ac JOIN selected_categories sc ON ac.category_id=sc.id WHERE ac.asset_id=a.id))
    AND NOT EXISTS(SELECT 1 FROM json_each(?3) wanted WHERE NOT EXISTS(SELECT 1 FROM asset_tags at WHERE at.asset_id=a.id AND at.tag_id=wanted.value))
    AND (?6=0 OR NOT EXISTS(SELECT 1 FROM asset_categories WHERE asset_id=a.id))
    AND (?7 IS NULL OR a.created_at<?7 OR (a.created_at=?7 AND a.id<?8))
    AND (?9=0 OR (NOT EXISTS(SELECT 1 FROM asset_documents WHERE asset_id=a.id AND kind='edited') AND NOT EXISTS(SELECT 1 FROM asset_notes WHERE asset_id=a.id)))
ORDER BY a.created_at DESC,a.id DESC
