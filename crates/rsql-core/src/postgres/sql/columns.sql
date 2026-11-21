SELECT c.relname::text,
       a.attname::text,
       format_type(a.atttypid, a.atttypmod),
       NOT (a.attnotnull OR (t.typtype = 'd' AND t.typnotnull)),
       CASE WHEN a.attgenerated = '' THEN pg_get_expr(d.adbin, d.adrelid) END,
       COALESCE(pk.is_pk, false),
       fk.target_schema,
       fk.target_table,
       fk.target_column
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
JOIN pg_attribute a ON a.attrelid = c.oid
JOIN pg_type t ON t.oid = a.atttypid
LEFT JOIN pg_attrdef d ON d.adrelid = c.oid AND d.adnum = a.attnum
LEFT JOIN LATERAL (
    SELECT true AS is_pk
    FROM pg_constraint pc
    WHERE pc.conrelid = c.oid AND pc.contype = 'p' AND a.attnum = ANY (pc.conkey)
    LIMIT 1
) pk ON true
LEFT JOIN LATERAL (
    SELECT fn.nspname::text AS target_schema,
           fc.relname::text AS target_table,
           fa.attname::text AS target_column
    FROM pg_constraint pc
    JOIN pg_class fc ON fc.oid = pc.confrelid
    JOIN pg_namespace fn ON fn.oid = fc.relnamespace
    JOIN pg_attribute fa ON fa.attrelid = pc.confrelid AND fa.attnum = pc.confkey[1]
    WHERE pc.conrelid = c.oid
      AND pc.contype = 'f'
      AND array_length(pc.conkey, 1) = 1
      AND pc.conkey[1] = a.attnum
    LIMIT 1
) fk ON true
WHERE n.nspname = $1
  AND c.relkind IN ('r', 'p', 'f', 'v', 'm')
  AND a.attnum > 0
  AND NOT a.attisdropped
ORDER BY c.relname, a.attnum
LIMIT $2
