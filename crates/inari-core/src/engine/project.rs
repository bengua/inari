//! The thread's folder as a project (stub until phase 4).

use super::LocalEngine;
use crate::agent::{Runner, Turn};
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;

impl LocalEngine {
    pub(crate) async fn project_action(
        &self,
        _runner: &Arc<Runner>,
        _agent: &str,
        action: &str,
        _p: &Value,
    ) -> Result<(), String> {
        Err(format!("unknown action {action:?}"))
    }
    pub(crate) async fn fill_project(&self, _agent: &str, _workdir: &Path) {}
    pub(crate) async fn project_tool(
        &self,
        _t: &Turn,
        name: &str,
        _args: &Value,
    ) -> Result<String, String> {
        Err(format!("{name} is not available yet"))
    }
}
