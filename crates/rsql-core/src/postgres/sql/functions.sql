SELECT p.proname::text,
       pg_catalog.pg_get_function_arguments(p.oid),
       pg_catalog.pg_get_function_result(p.oid),
       p.prokind::text,
       p.oid,
       p.proargtypes::pg_catalog.oid[]
FROM pg_catalog.pg_proc p
JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace
WHERE n.nspname = $1
  AND p.prokind IN ('f', 'a', 'w')
ORDER BY p.proname, p.oid
