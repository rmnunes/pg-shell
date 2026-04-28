use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Schema {
    pub name: String,
    pub owner: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    Table,
    View,
    MaterializedView,
    PartitionedTable,
    ForeignTable,
}

impl RelationKind {
    pub(crate) fn from_relkind(c: char) -> Option<Self> {
        match c {
            'r' => Some(Self::Table),
            'v' => Some(Self::View),
            'm' => Some(Self::MaterializedView),
            'p' => Some(Self::PartitionedTable),
            'f' => Some(Self::ForeignTable),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Relation {
    pub schema: String,
    pub name: String,
    pub kind: RelationKind,
    pub oid: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Column {
    pub name: String,
    pub ord: i16,
    pub type_name: String,
    pub not_null: bool,
    pub default_expr: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Function {
    pub schema: String,
    pub name: String,
    pub args: String,
    pub result: String,
    /// `f` = function, `p` = procedure, `a` = aggregate, `w` = window.
    pub kind: char,
}

#[derive(Debug, Clone, Serialize)]
pub struct Sequence {
    pub schema: String,
    pub name: String,
    pub owner: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Trigger {
    pub schema: String,
    pub table: String,
    pub name: String,
    /// `BEFORE` | `AFTER` | `INSTEAD OF`.
    pub timing: String,
    /// Space-separated event list: `INSERT`, `UPDATE`, `DELETE`, `TRUNCATE`,
    /// joined with ` OR ` when multiple.
    pub event: String,
    /// Full `CREATE TRIGGER …` definition for the "View Definition" action.
    pub definition: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Index {
    pub schema: String,
    pub table: String,
    pub name: String,
    pub is_unique: bool,
    pub is_primary: bool,
    /// Full `CREATE INDEX …` definition.
    pub definition: String,
}

/// Used by the scripting / definition commands to discriminate.
#[derive(Debug, Clone, Copy, Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    Table,
    View,
    MaterializedView,
    Function,
    Procedure,
    Sequence,
    Trigger,
    Index,
}
