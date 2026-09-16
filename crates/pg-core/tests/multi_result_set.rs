//! Live-database checks for batches that return several result sets.
//!
//! Ignored by default because they need a server. Point `PG_SHELL_TEST_URL` at
//! a throwaway database and run:
//!
//! ```sh
//! PG_SHELL_TEST_URL=postgres://user:pw@localhost/scratch \
//!     cargo test -p pg-core --test multi_result_set -- --ignored
//! ```
//!
//! The regression these pin: `execute_streaming` used to announce columns once,
//! on the first row-producing statement, and stream every later statement's
//! rows under that same header. A script whose SELECTs have different shapes
//! then fed 2-column rows to a 4-column grid, which threw in the renderer and
//! took the whole window blank.

use pg_core::{execute_streaming, QueryStart};
use sqlx::postgres::PgPoolOptions;

fn url() -> Option<String> {
    std::env::var("PG_SHELL_TEST_URL").ok()
}

/// Runs `sql`, returning each result set's column names and the row counts
/// recorded against each `result_index`.
async fn run(sql: &str) -> (Vec<Vec<String>>, Vec<usize>) {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url().expect("PG_SHELL_TEST_URL"))
        .await
        .expect("connect");

    let starts: std::sync::Arc<std::sync::Mutex<Vec<QueryStart>>> = Default::default();
    let rows: std::sync::Arc<std::sync::Mutex<Vec<(u32, usize)>>> = Default::default();

    let s = starts.clone();
    let r = rows.clone();
    execute_streaming(
        pool,
        sql.to_string(),
        move |start| s.lock().unwrap().push(start),
        move |result_index, batch| r.lock().unwrap().push((result_index, batch.len())),
        |_cmd| {},
    )
    .await
    .expect("execute");

    let starts = starts.lock().unwrap();
    let names: Vec<Vec<String>> = starts
        .iter()
        .map(|s| s.columns.iter().map(|c| c.name.clone()).collect())
        .collect();

    let mut counts = vec![0usize; starts.len()];
    for (idx, n) in rows.lock().unwrap().iter() {
        if let Some(slot) = counts.get_mut(*idx as usize) {
            *slot += n;
        }
    }
    (names, counts)
}

#[tokio::test]
#[ignore = "needs PG_SHELL_TEST_URL"]
async fn each_select_gets_its_own_columns() {
    let (names, counts) = run(
        "SELECT 1 AS a; \
         SELECT 2 AS b, 3 AS c, 4 AS d, 5 AS e; \
         SELECT 6 AS f, 7 AS g;",
    )
    .await;

    assert_eq!(names.len(), 3, "one QueryStart per SELECT, got {names:?}");
    assert_eq!(names[0], ["a"]);
    assert_eq!(names[1], ["b", "c", "d", "e"]);
    assert_eq!(names[2], ["f", "g"]);
    assert_eq!(counts, vec![1, 1, 1], "each result set owns its own rows");
}

/// Non-row-returning statements in between must not burn a result index, or
/// rows land in a slot the UI never created columns for.
#[tokio::test]
#[ignore = "needs PG_SHELL_TEST_URL"]
async fn ddl_between_selects_does_not_consume_an_index() {
    let (names, counts) = run(
        "SELECT 1 AS first; \
         CREATE TEMP TABLE t_gap(x int); \
         INSERT INTO t_gap VALUES (1), (2); \
         SELECT x AS second FROM t_gap ORDER BY x;",
    )
    .await;

    assert_eq!(names.len(), 2, "only SELECTs open result sets, got {names:?}");
    assert_eq!(names[0], ["first"]);
    assert_eq!(names[1], ["second"]);
    assert_eq!(counts, vec![1, 2]);
}

/// A batch that returns nothing still announces once, with empty columns, so
/// the UI can render a command-only summary.
#[tokio::test]
#[ignore = "needs PG_SHELL_TEST_URL"]
async fn command_only_batch_still_announces_once() {
    let (names, _) = run("CREATE TEMP TABLE t_none(x int); DROP TABLE t_none;").await;
    assert_eq!(names.len(), 1);
    assert!(names[0].is_empty());
}

/// The psql-script case that started this: stripped `\echo` lines, then three
/// verification SELECTs of different widths.
#[tokio::test]
#[ignore = "needs PG_SHELL_TEST_URL"]
async fn psql_style_script_with_echo_and_mixed_selects() {
    let (names, counts) = run(
        "BEGIN;\n\
         CREATE TEMP TABLE t_perm(code text, category text, is_enabled bool);\n\
         INSERT INTO t_perm VALUES ('a','dashboard',true);\n\
         COMMIT;\n\
         \\echo ''\n\
         \\echo 'Permission codes:'\n\
         SELECT code, category, is_enabled FROM t_perm;\n\
         \\echo 'Role:'\n\
         SELECT 'SM' AS code, 'Sales Manager' AS label, true AS is_enabled, false AS is_system;\n\
         \\echo 'Grants:'\n\
         SELECT 'CM' AS role_code, 'dashboard.credit.view' AS permission_code;",
    )
    .await;

    assert_eq!(names.len(), 3, "three verification SELECTs, got {names:?}");
    assert_eq!(names[0].len(), 3);
    assert_eq!(names[1].len(), 4);
    assert_eq!(names[2].len(), 2);
    assert_eq!(counts, vec![1, 1, 1]);
}
