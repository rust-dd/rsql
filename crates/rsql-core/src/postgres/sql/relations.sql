SELECT c.relname::text,
       c.relkind::text,
       obj_description(c.oid, 'pg_class'),
       c.oid
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE n.nspname = $1
  AND c.relkind IN ('r', 'p', 'f', 'v', 'm')
ORDER BY c.relname
