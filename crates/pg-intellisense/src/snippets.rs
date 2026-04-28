//! Built-in snippet library. Offered at statement-initial position only, so
//! typing `ssf` inside a `WHERE` clause doesn't suggest `SELECT * FROM`.
//!
//! Snippet placeholders use Monaco's `${N:label}` syntax so the frontend can
//! set `insertTextRules = InsertAsSnippet` and get tab-through behavior.

pub struct Snippet {
    pub trigger: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub body: &'static str,
}

/// Triggers are intentionally short and Redgate-flavored.
pub const BUILTIN: &[Snippet] = &[
    Snippet {
        trigger: "ssf",
        label: "ssf — SELECT * FROM",
        description: "SELECT * FROM …",
        body: "SELECT *\nFROM ${1:table}$0",
    },
    Snippet {
        trigger: "sf",
        label: "sf — SELECT ... FROM",
        description: "SELECT columns FROM …",
        body: "SELECT ${1:columns}\nFROM ${2:table}\nWHERE ${3:condition}$0",
    },
    Snippet {
        trigger: "ct",
        label: "ct — CREATE TABLE",
        description: "CREATE TABLE skeleton",
        body: "CREATE TABLE ${1:name} (\n    id bigserial PRIMARY KEY,\n    ${2:col} ${3:type} NOT NULL,\n    created_at timestamptz NOT NULL DEFAULT now()\n);$0",
    },
    Snippet {
        trigger: "ctas",
        label: "ctas — CREATE TABLE AS",
        description: "CREATE TABLE AS SELECT",
        body: "CREATE TABLE ${1:name} AS\nSELECT ${2:*}\nFROM ${3:source}$0",
    },
    Snippet {
        trigger: "j",
        label: "j — JOIN ... ON",
        description: "JOIN target ON condition",
        body: "JOIN ${1:table} ${2:alias} ON ${3:condition}$0",
    },
    Snippet {
        trigger: "lj",
        label: "lj — LEFT JOIN",
        description: "LEFT JOIN target ON condition",
        body: "LEFT JOIN ${1:table} ${2:alias} ON ${3:condition}$0",
    },
    Snippet {
        trigger: "ij",
        label: "ij — INNER JOIN",
        description: "INNER JOIN target ON condition",
        body: "INNER JOIN ${1:table} ${2:alias} ON ${3:condition}$0",
    },
    Snippet {
        trigger: "cte",
        label: "cte — WITH block",
        description: "WITH cte AS (...) SELECT",
        body: "WITH ${1:cte} AS (\n    SELECT ${2:*}\n    FROM ${3:source}\n)\nSELECT *\nFROM ${1:cte}$0",
    },
    Snippet {
        trigger: "ins",
        label: "ins — INSERT",
        description: "INSERT INTO … VALUES",
        body: "INSERT INTO ${1:table} (${2:cols})\nVALUES (${3:values})$0",
    },
    Snippet {
        trigger: "upd",
        label: "upd — UPDATE",
        description: "UPDATE … SET … WHERE",
        body: "UPDATE ${1:table}\nSET ${2:col} = ${3:value}\nWHERE ${4:condition}$0",
    },
    Snippet {
        trigger: "del",
        label: "del — DELETE",
        description: "DELETE FROM … WHERE",
        body: "DELETE FROM ${1:table}\nWHERE ${2:condition}$0",
    },
    Snippet {
        trigger: "win",
        label: "win — window function",
        description: "OVER (PARTITION BY … ORDER BY …)",
        body: "${1:row_number}() OVER (PARTITION BY ${2:col} ORDER BY ${3:col})$0",
    },
    Snippet {
        trigger: "case",
        label: "case — CASE WHEN",
        description: "CASE WHEN … THEN … ELSE … END",
        body: "CASE\n    WHEN ${1:cond} THEN ${2:result}\n    ELSE ${3:default}\nEND$0",
    },
];
