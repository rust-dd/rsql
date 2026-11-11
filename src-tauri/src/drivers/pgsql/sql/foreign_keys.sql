SELECT src.relname,
       a_src.attname,
       tgt_nsp.nspname,
       tgt.relname,
       a_tgt.attname
FROM pg_constraint con
JOIN pg_class src ON src.oid = con.conrelid
JOIN pg_namespace src_nsp ON src_nsp.oid = src.relnamespace
JOIN pg_class tgt ON tgt.oid = con.confrelid
JOIN pg_namespace tgt_nsp ON tgt_nsp.oid = tgt.relnamespace
CROSS JOIN LATERAL unnest(con.conkey, con.confkey) AS k(src_attnum, tgt_attnum)
JOIN pg_attribute a_src ON a_src.attrelid = con.conrelid AND a_src.attnum = k.src_attnum
JOIN pg_attribute a_tgt ON a_tgt.attrelid = con.confrelid AND a_tgt.attnum = k.tgt_attnum
WHERE con.contype = 'f'
  AND src_nsp.nspname = $1
  -- A key into a partitioned table also gets an internal copy per partition.
  AND NOT EXISTS (
      SELECT 1 FROM pg_constraint parent
      WHERE parent.oid = con.conparentid AND parent.conrelid = con.conrelid
  )
-- The results grid follows the first key listed for a column.
ORDER BY src.relname, a_src.attname, cardinality(con.conkey), con.conname
