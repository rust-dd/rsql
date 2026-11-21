pub(super) const SNIPPETS: &[(&str, &str, &str)] = &[
    (
        "sel",
        "SELECT ... FROM ... WHERE",
        "SELECT ${1:*}\nFROM ${2:table_name}\nWHERE ${3:condition}\nLIMIT ${4:100};",
    ),
    (
        "selc",
        "SELECT COUNT(*)",
        "SELECT COUNT(*)\nFROM ${1:table_name}\nWHERE ${2:1=1};",
    ),
    (
        "seld",
        "SELECT DISTINCT",
        "SELECT DISTINCT ${1:column}\nFROM ${2:table_name};",
    ),
    (
        "ins",
        "INSERT INTO ... VALUES",
        "INSERT INTO ${1:table_name} (${2:columns})\nVALUES (${3:values});",
    ),
    (
        "upd",
        "UPDATE ... SET ... WHERE",
        "UPDATE ${1:table_name}\nSET ${2:column} = ${3:value}\nWHERE ${4:condition};",
    ),
    (
        "del",
        "DELETE FROM ... WHERE",
        "DELETE FROM ${1:table_name}\nWHERE ${2:condition};",
    ),
    (
        "crt",
        "CREATE TABLE",
        "CREATE TABLE ${1:table_name} (\n  ${2:id} SERIAL PRIMARY KEY,\n  ${3:column} ${4:TEXT} NOT NULL\n);",
    ),
    (
        "alt",
        "ALTER TABLE ADD COLUMN",
        "ALTER TABLE ${1:table_name}\nADD COLUMN ${2:column_name} ${3:TEXT};",
    ),
    (
        "idx",
        "CREATE INDEX",
        "CREATE INDEX ${1:idx_name}\nON ${2:table_name} (${3:column});",
    ),
    (
        "jn",
        "SELECT ... JOIN ... ON",
        "SELECT ${1:*}\nFROM ${2:table1} t1\nJOIN ${3:table2} t2 ON t1.${4:id} = t2.${5:t1_id}\nWHERE ${6:1=1};",
    ),
    (
        "lj",
        "SELECT ... LEFT JOIN",
        "SELECT ${1:*}\nFROM ${2:table1} t1\nLEFT JOIN ${3:table2} t2 ON t1.${4:id} = t2.${5:t1_id};",
    ),
    (
        "grp",
        "SELECT ... GROUP BY ... ORDER BY",
        "SELECT ${1:column}, COUNT(*)\nFROM ${2:table_name}\nGROUP BY ${1:column}\nORDER BY COUNT(*) DESC;",
    ),
    (
        "cte",
        "WITH ... AS (...) SELECT",
        "WITH ${1:cte_name} AS (\n  SELECT ${2:*}\n  FROM ${3:table_name}\n  WHERE ${4:condition}\n)\nSELECT * FROM ${1:cte_name};",
    ),
    (
        "exist",
        "SELECT ... WHERE EXISTS",
        "SELECT *\nFROM ${1:table_name} t1\nWHERE EXISTS (\n  SELECT 1\n  FROM ${2:other_table} t2\n  WHERE t2.${3:fk} = t1.${4:id}\n);",
    ),
    (
        "upsert",
        "INSERT ... ON CONFLICT DO UPDATE",
        "INSERT INTO ${1:table_name} (${2:columns})\nVALUES (${3:values})\nON CONFLICT (${4:constraint})\nDO UPDATE SET ${5:column} = EXCLUDED.${5:column};",
    ),
    (
        "vw",
        "CREATE VIEW",
        "CREATE OR REPLACE VIEW ${1:view_name} AS\nSELECT ${2:*}\nFROM ${3:table_name}\nWHERE ${4:condition};",
    ),
    (
        "fn",
        "CREATE FUNCTION",
        "CREATE OR REPLACE FUNCTION ${1:func_name}(${2:params})\nRETURNS ${3:return_type}\nLANGUAGE plpgsql\nAS \\$\\$\nBEGIN\n  ${4:-- body}\nEND;\n\\$\\$;",
    ),
    (
        "trg",
        "CREATE TRIGGER",
        "CREATE TRIGGER ${1:trigger_name}\n${2:BEFORE} ${3:INSERT} ON ${4:table_name}\nFOR EACH ROW\nEXECUTE FUNCTION ${5:func_name}();",
    ),
    (
        "txn",
        "BEGIN ... COMMIT",
        "BEGIN;\n  ${1:-- statements}\nCOMMIT;",
    ),
];
