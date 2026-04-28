use tauri::State;

use crate::error::{AppError, AppResult};
use crate::history::HistoryEntry;
use crate::state::AppState;

/// Most-recent-first list. `search` is a substring match on the SQL text.
/// Defaults: 100 entries, no filter.
#[tauri::command]
pub async fn history_list(
    profile_id: String,
    limit: Option<i64>,
    search: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<Vec<HistoryEntry>> {
    let lim = limit.unwrap_or(100).clamp(1, 1000);
    state
        .history
        .list(&profile_id, lim, search.as_deref())
        .map_err(|e| AppError::new("history", e.to_string()))
}

#[tauri::command]
pub async fn history_clear(profile_id: String, state: State<'_, AppState>) -> AppResult<u64> {
    state
        .history
        .clear(&profile_id)
        .map_err(|e| AppError::new("history", e.to_string()))
}
