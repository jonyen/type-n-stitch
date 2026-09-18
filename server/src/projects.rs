//! Projects, membership and roles.

use std::sync::Arc;

use crate::error::AppResult;
use crate::AppState;

/// Give every media directory that has no project yet to `owner_id`.
pub async fn adopt_orphans(_state: &Arc<AppState>, _owner_id: &str) -> AppResult<()> {
    Ok(())
}
