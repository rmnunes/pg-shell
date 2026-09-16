//! psql meta-command handling for pasted scripts.
//!
//! pg-shell talks to Postgres directly over the wire; it is not psql. A
//! backslash command like `\echo` is interpreted by psql itself and never
//! reaches the server, so sending a script containing one produces the
//! server's unhelpful `syntax error at or near "\"` with a position pointing
//! at a line the author considers perfectly valid.
//!
//! Operations scripts are written for psql and routinely carry a few of these.
//! Rather than reject the whole batch, strip the ones that only affect psql's
//! own output — the SQL then runs unchanged — and fail with a precise message
//! naming the command and line for the ones that would change what the script
//! *does*. Silently ignoring `\set` or `\gset` would run a different script
//! than the author wrote, which is worse than refusing.
//!
//! Stripped lines are replaced by empty lines rather than removed, so byte and
//! line positions in any later server error still line up with what the user
//! sees in the editor.

/// Meta-commands that change only psql's presentation, never the statements
/// executed or their results. Safe to drop.
///
/// Case matters here exactly as it does in psql: `\C` sets the table title and
/// is listed, while `\c` is `\connect` and must never be dropped silently —
/// that would run the rest of the script against a different database than the
/// author wrote it for.
const OUTPUT_ONLY: &[&str] = &[
    "C", "H", "a", "echo", "f", "h", "pset", "qecho", "t", "timing", "warn", "x",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PsqlMetaCommand {
    /// 1-based line number, matching what an editor shows.
    pub line: usize,
    /// The command without its backslash, e.g. `gset`.
    pub name: String,
}

impl PsqlMetaCommand {
    pub fn message(&self) -> String {
        format!(
            "line {}: psql meta-command \\{} is not supported. pg-shell connects to \
             Postgres directly and cannot run psql client commands. Remove the line, \
             or run this script with psql.",
            self.line, self.name
        )
    }
}

/// Tracks whether a given line begins inside a construct where a leading
/// backslash is ordinary text rather than a meta-command.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scan {
    Code,
    BlockComment(u32),
    Single,
    Double,
    Dollar,
}

/// Walk `line` and return the scanner state the *next* line starts in.
///
/// Only the transitions that can span a newline matter here: quotes, dollar
/// quoting and block comments. `$tag$` bodies are treated as one opaque region
/// because a tag cannot contain a newline, so any `$...$` that closes it is
/// found on some later line by the same rule.
fn advance(line: &str, mut state: Scan, dollar_tag: &mut String) -> Scan {
    let bytes = line.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let c = bytes[i];
        match state {
            Scan::BlockComment(depth) => {
                if bytes[i..].starts_with(b"*/") {
                    state = if depth <= 1 {
                        Scan::Code
                    } else {
                        Scan::BlockComment(depth - 1)
                    };
                    i += 2;
                    continue;
                }
                if bytes[i..].starts_with(b"/*") {
                    state = Scan::BlockComment(depth + 1);
                    i += 2;
                    continue;
                }
            }
            Scan::Single => {
                // '' is an escaped quote, not a close.
                if c == b'\'' {
                    if bytes.get(i + 1) == Some(&b'\'') {
                        i += 2;
                        continue;
                    }
                    state = Scan::Code;
                }
            }
            Scan::Double => {
                if c == b'"' {
                    if bytes.get(i + 1) == Some(&b'"') {
                        i += 2;
                        continue;
                    }
                    state = Scan::Code;
                }
            }
            Scan::Dollar => {
                if c == b'$' {
                    if let Some(end) = find_dollar_tag(&bytes[i..]) {
                        if &bytes[i..i + end] == dollar_tag.as_bytes() {
                            dollar_tag.clear();
                            state = Scan::Code;
                            i += end;
                            continue;
                        }
                    }
                }
            }
            Scan::Code => {
                if bytes[i..].starts_with(b"--") {
                    return Scan::Code; // rest of line is a comment
                }
                if bytes[i..].starts_with(b"/*") {
                    state = Scan::BlockComment(1);
                    i += 2;
                    continue;
                }
                match c {
                    b'\'' => state = Scan::Single,
                    b'"' => state = Scan::Double,
                    b'$' => {
                        if let Some(end) = find_dollar_tag(&bytes[i..]) {
                            *dollar_tag = String::from_utf8_lossy(&bytes[i..i + end]).into_owned();
                            state = Scan::Dollar;
                            i += end;
                            continue;
                        }
                    }
                    _ => {}
                }
            }
        }
        i += 1;
    }

    state
}

/// If `bytes` opens with a dollar-quote tag (`$$` or `$name$`), return its
/// length in bytes. Tags are `$`, an optional identifier, then `$`.
fn find_dollar_tag(bytes: &[u8]) -> Option<usize> {
    debug_assert_eq!(bytes.first(), Some(&b'$'));
    let mut i = 1;
    while i < bytes.len() {
        match bytes[i] {
            b'$' => return Some(i + 1),
            c if c.is_ascii_alphanumeric() || c == b'_' => i += 1,
            _ => return None,
        }
    }
    None
}

/// Remove psql meta-commands that only affect psql's output.
///
/// Returns the rewritten SQL, or the first unsupported meta-command found.
pub fn strip_psql_meta(sql: &str) -> Result<String, PsqlMetaCommand> {
    // Cheap bail-out: the overwhelming majority of statements have no
    // backslash at all, and this runs on every execution.
    if !sql.contains('\\') {
        return Ok(sql.to_string());
    }

    let mut out = String::with_capacity(sql.len());
    let mut state = Scan::Code;
    let mut dollar_tag = String::new();
    let mut first = true;

    for (idx, line) in sql.split('\n').enumerate() {
        if !first {
            out.push('\n');
        }
        first = false;

        // A meta-command is only a meta-command at the start of a line, and
        // only when that line starts in ordinary SQL.
        let stripped = line.strip_suffix('\r').unwrap_or(line);
        let is_meta = state == Scan::Code
            && stripped
                .trim_start()
                .strip_prefix('\\')
                .is_some_and(|rest| !rest.starts_with('\\'));

        if is_meta {
            let rest = stripped.trim_start().trim_start_matches('\\');
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '?' || *c == '!')
                .collect();
            let name = if name.is_empty() {
                rest.chars().next().map(String::from).unwrap_or_default()
            } else {
                name
            };

            if OUTPUT_ONLY.contains(&name.as_str()) {
                // Blank line keeps later error positions aligned with the editor.
                continue;
            }
            return Err(PsqlMetaCommand {
                line: idx + 1,
                name,
            });
        }

        out.push_str(line);
        state = advance(stripped, state, &mut dollar_tag);
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_plain_sql_through_untouched() {
        let sql = "SELECT 1;\nSELECT 2;";
        assert_eq!(strip_psql_meta(sql).unwrap(), sql);
    }

    #[test]
    fn strips_echo_and_keeps_line_count() {
        let sql = "SELECT 1;\n\\echo 'hello'\nSELECT 2;";
        let out = strip_psql_meta(sql).unwrap();
        assert_eq!(out, "SELECT 1;\n\nSELECT 2;");
        assert_eq!(out.lines().count(), sql.lines().count());
    }

    #[test]
    fn strips_leading_whitespace_meta() {
        let sql = "SELECT 1;\n   \\timing on\nSELECT 2;";
        assert_eq!(strip_psql_meta(sql).unwrap(), "SELECT 1;\n\nSELECT 2;");
    }

    #[test]
    fn rejects_result_changing_meta_with_line_number() {
        let sql = "SELECT 1;\n\\echo hi\n\\gset\nSELECT 2;";
        let err = strip_psql_meta(sql).unwrap_err();
        assert_eq!(err.line, 3);
        assert_eq!(err.name, "gset");
        assert!(err.message().contains("\\gset"));
        assert!(err.message().contains("line 3"));
    }

    #[test]
    fn rejects_result_changing_commands() {
        for (sql, name) in [
            ("\\i other.sql", "i"),
            ("\\ir other.sql", "ir"),
            ("\\gexec", "gexec"),
            ("\\dt", "dt"),
            ("\\set v 1", "set"),
            ("\\copy t FROM 'f.csv'", "copy"),
            ("\\watch 5", "watch"),
        ] {
            assert_eq!(strip_psql_meta(sql).unwrap_err().name, name, "for {sql}");
        }
    }

    /// `\c` switches database. Stripping it would silently run the rest of the
    /// script somewhere else, so it is refused even though `\C` is dropped.
    #[test]
    fn connect_is_rejected_but_title_is_stripped() {
        assert_eq!(strip_psql_meta("\\c otherdb").unwrap_err().name, "c");
        assert_eq!(
            strip_psql_meta("\\connect otherdb").unwrap_err().name,
            "connect"
        );
        assert_eq!(strip_psql_meta("\\C 'My title'").unwrap(), "");
    }

    #[test]
    fn ignores_backslash_inside_single_quotes() {
        let sql = "SELECT 'a\nb\\echo not a command\nc';";
        assert_eq!(strip_psql_meta(sql).unwrap(), sql);
    }

    #[test]
    fn ignores_backslash_inside_dollar_quotes() {
        let sql = "CREATE FUNCTION f() RETURNS void AS $$\n\\echo nope\n$$ LANGUAGE sql;";
        assert_eq!(strip_psql_meta(sql).unwrap(), sql);
    }

    #[test]
    fn ignores_backslash_inside_tagged_dollar_quotes() {
        let sql = "DO $body$\n\\gset\n$body$;";
        assert_eq!(strip_psql_meta(sql).unwrap(), sql);
    }

    #[test]
    fn ignores_backslash_inside_block_comment() {
        let sql = "/*\n\\gset\n*/\nSELECT 1;";
        assert_eq!(strip_psql_meta(sql).unwrap(), sql);
    }

    #[test]
    fn handles_mid_line_backslash_in_string() {
        let sql = "SELECT 'C:\\dev\\pg-shell';";
        assert_eq!(strip_psql_meta(sql).unwrap(), sql);
    }

    #[test]
    fn line_comment_does_not_leak_state() {
        let sql = "-- a 'quote\nSELECT 1;\n\\echo done";
        assert_eq!(strip_psql_meta(sql).unwrap(), "-- a 'quote\nSELECT 1;\n");
    }

    #[test]
    fn preserves_crlf_bodies() {
        let sql = "SELECT 1;\r\n\\echo hi\r\nSELECT 2;";
        let out = strip_psql_meta(sql).unwrap();
        assert!(out.starts_with("SELECT 1;\r\n"));
        assert!(out.ends_with("SELECT 2;"));
    }

    #[test]
    fn escaped_quote_does_not_end_string() {
        let sql = "SELECT 'it''s\n\\gset\nfine';";
        assert_eq!(strip_psql_meta(sql).unwrap(), sql);
    }
}
