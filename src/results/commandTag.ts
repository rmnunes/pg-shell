// Derive a human-readable tag for each top-level statement in a SQL buffer.
// Walks the string respecting single-quoted strings, dollar-quoted strings,
// and both PG comment styles so commas and semicolons inside them don't
// split statements. For each statement, the first bareword keyword
// (uppercased) becomes the tag ("SELECT", "INSERT", "UPDATE", "TRUNCATE",
// "CREATE TABLE", etc.).
export function deriveCommandTags(sql: string): string[] {
  const statements = splitStatements(sql);
  return statements.map(firstKeywordPhrase);
}

function splitStatements(sql: string): string[] {
  const out: string[] = [];
  let i = 0;
  let start = 0;
  while (i < sql.length) {
    const c = sql[i];
    // Line comment
    if (c === "-" && sql[i + 1] === "-") {
      while (i < sql.length && sql[i] !== "\n") i++;
      continue;
    }
    // Block comment (nestable)
    if (c === "/" && sql[i + 1] === "*") {
      i += 2;
      let depth = 1;
      while (i < sql.length && depth > 0) {
        if (sql[i] === "/" && sql[i + 1] === "*") {
          depth++;
          i += 2;
        } else if (sql[i] === "*" && sql[i + 1] === "/") {
          depth--;
          i += 2;
        } else {
          i++;
        }
      }
      continue;
    }
    // Single-quoted string
    if (c === "'") {
      i++;
      while (i < sql.length) {
        if (sql[i] === "'") {
          if (sql[i + 1] === "'") i += 2;
          else {
            i++;
            break;
          }
        } else i++;
      }
      continue;
    }
    // Dollar-quoted string
    if (c === "$") {
      const tagMatch = sql.slice(i).match(/^\$([A-Za-z_0-9]*)\$/);
      if (tagMatch) {
        const term = `$${tagMatch[1]}$`;
        const end = sql.indexOf(term, i + term.length);
        i = end < 0 ? sql.length : end + term.length;
        continue;
      }
    }
    if (c === ";") {
      out.push(sql.slice(start, i));
      start = i + 1;
    }
    i++;
  }
  if (start < sql.length) {
    const tail = sql.slice(start);
    if (tail.trim().length) out.push(tail);
  }
  return out;
}

function firstKeywordPhrase(stmt: string): string {
  // Strip leading comments & whitespace.
  let s = stmt;
  let prev;
  do {
    prev = s;
    s = s.replace(/^\s+/, "");
    s = s.replace(/^--[^\n]*\n?/, "");
    s = s.replace(/^\/\*[\s\S]*?\*\//, "");
  } while (s !== prev);
  const m = s.match(/^[A-Za-z_][A-Za-z_0-9]*/);
  if (!m) return "SQL";
  const first = m[0].toUpperCase();
  // Two-word phrases we recognize — helps render "CREATE TABLE" etc.
  const rest = s.slice(m[0].length).replace(/^\s+/, "");
  const next = rest.match(/^[A-Za-z_][A-Za-z_0-9]*/);
  if (next) {
    const phrase = `${first} ${next[0].toUpperCase()}`;
    if (TWO_WORD.has(phrase)) return phrase;
  }
  return first;
}

const TWO_WORD = new Set([
  "CREATE TABLE",
  "CREATE INDEX",
  "CREATE VIEW",
  "CREATE MATERIALIZED",
  "CREATE FUNCTION",
  "CREATE PROCEDURE",
  "CREATE SCHEMA",
  "CREATE ROLE",
  "CREATE SEQUENCE",
  "CREATE EXTENSION",
  "CREATE TYPE",
  "CREATE TRIGGER",
  "DROP TABLE",
  "DROP INDEX",
  "DROP VIEW",
  "DROP FUNCTION",
  "DROP SCHEMA",
  "DROP ROLE",
  "DROP SEQUENCE",
  "DROP EXTENSION",
  "DROP TYPE",
  "DROP TRIGGER",
  "ALTER TABLE",
  "ALTER INDEX",
  "ALTER VIEW",
  "ALTER FUNCTION",
  "ALTER SCHEMA",
  "ALTER ROLE",
  "ALTER SEQUENCE",
  "ALTER TYPE",
  "DELETE FROM",
  "INSERT INTO",
  "REFRESH MATERIALIZED",
]);
