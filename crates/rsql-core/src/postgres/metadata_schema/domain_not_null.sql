EXISTS (
    WITH RECURSIVE bases AS (
        SELECT t.typnotnull, t.typbasetype
        UNION ALL
        SELECT b.typnotnull, b.typbasetype
        FROM bases JOIN pg_catalog.pg_type b ON b.oid = bases.typbasetype
    )
    SELECT FROM bases WHERE typnotnull
)
