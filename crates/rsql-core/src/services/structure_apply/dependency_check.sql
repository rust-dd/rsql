WITH RECURSIVE roots(classid, objid, objsubid) AS (
    SELECT 'pg_catalog.pg_class'::pg_catalog.regclass::oid, $1::oid, n::integer FROM pg_catalog.unnest($2::smallint[]) n
    UNION ALL SELECT 'pg_catalog.pg_constraint'::pg_catalog.regclass::oid, n, 0 FROM pg_catalog.unnest($3::oid[]) n
    UNION ALL SELECT 'pg_catalog.pg_class'::pg_catalog.regclass::oid, n, 0 FROM pg_catalog.unnest($4::oid[]) n
), affected(classid, objid, objsubid) AS (
    SELECT * FROM roots
    UNION
    SELECT d.classid, d.objid, d.objsubid FROM affected a
    JOIN pg_catalog.pg_depend d ON d.refclassid = a.classid AND d.refobjid = a.objid
        AND (a.objsubid = 0 OR d.refobjsubid = a.objsubid)
)
SELECT pg_catalog.pg_describe_object(a.classid, a.objid, a.objsubid)
FROM affected a
WHERE NOT EXISTS (SELECT FROM roots r WHERE (r.classid, r.objid, r.objsubid) = (a.classid, a.objid, a.objsubid))
AND NOT (
    a.classid = 'pg_catalog.pg_attrdef'::pg_catalog.regclass
    AND EXISTS (SELECT FROM pg_catalog.pg_attrdef v WHERE v.oid = a.objid AND v.adrelid = $1
        AND v.adnum = ANY($2::smallint[]))
)
AND NOT (
    a.classid = 'pg_catalog.pg_trigger'::pg_catalog.regclass
    AND EXISTS (SELECT FROM pg_catalog.pg_trigger t WHERE t.oid = a.objid AND t.tgisinternal AND t.tgconstraint = ANY($3::oid[]))
)
ORDER BY 1 LIMIT 1
