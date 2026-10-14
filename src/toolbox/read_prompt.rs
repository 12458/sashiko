// Copyright 2026 The Sashiko Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::toolbox::SashikoToolContext;
use crate::toolbox::framework::LlmTool;
use crate::toolbox::utils::validate_path;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::fs;

pub struct ReadPromptTool;

#[async_trait]
impl LlmTool<SashikoToolContext> for ReadPromptTool {
    fn name(&self) -> &'static str {
        "read_prompt"
    }

    fn description(&self) -> &'static str {
        "Read a specific prompt file from the prompt registry (e.g., 'mm.md', 'locking.md')."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Name of the prompt file (e.g., 'patterns/BPF-001.md')." }
            },
            "required": ["name"]
        })
    }

    async fn call(&self, args: Value, context: &SashikoToolContext) -> Result<Value> {
        let prompts_path = context
            .prompts_path
            .as_ref()
            .ok_or_else(|| anyhow!("read_prompt tool is not available"))?;
        let name = args["name"]
            .as_str()
            .ok_or_else(|| anyhow!("Missing prompt name"))?;

        let path = resolve_prompt_path(name, prompts_path)?;
        let content = fs::read_to_string(path).await?;

        Ok(json!({ "content": content }))
    }
}

fn resolve_prompt_path(name: &str, prompts_path: &std::path::Path) -> Result<std::path::PathBuf> {
    let normalized = crate::workflows::guard::normalize_prompt_relpath(name);
    let primary = validate_path(normalized, prompts_path)?;
    if primary.is_file() || normalized.contains('/') {
        return Ok(primary);
    }
    for subdir in ["subsystem", "patterns"] {
        let candidate_rel = format!("{subdir}/{normalized}");
        if let Ok(candidate) = validate_path(&candidate_rel, prompts_path)
            && candidate.is_file()
        {
            return Ok(candidate);
        }
    }
    Ok(primary)
}
