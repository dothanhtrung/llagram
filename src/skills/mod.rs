//! Built-in LLM skills. Add a new skill by:
//! 1. Creating `src/skills/<name>.rs` and registering it here
//! 2. Adding `prompts/SKILL_<NAME>.md` (read at runtime, no rebuild)

mod web;

use serde_json::Value;
use std::collections::HashSet;

#[derive(Clone)]
pub struct Registry {
    web: web::WebSkill,
}

#[derive(Clone, Copy)]
pub struct SkillInfo {
    pub name: &'static str,
    pub description: &'static str,
}

impl Registry {
    pub fn builtin() -> Result<Self, String> {
        Ok(Self {
            web: web::WebSkill::new()?,
        })
    }

    pub fn all() -> &'static [SkillInfo] {
        &[SkillInfo {
            name: "web",
            description: "Search the web and fetch page text",
        }]
    }

    pub fn canonical_name(name: &str) -> Option<&'static str> {
        Self::all()
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
            .map(|s| s.name)
    }

    pub fn tools_for(&self, enabled: &HashSet<String>) -> Vec<Value> {
        let mut tools = Vec::new();
        if enabled.iter().any(|s| s == "web") {
            tools.extend(web::tools());
        }
        tools
    }

    pub async fn execute(&self, tool: &str, args: Value) -> Result<String, String> {
        match tool {
            "web_search" | "web_fetch" => self.web.execute(tool, args).await,
            other => Err(format!("unknown tool '{other}'")),
        }
    }
}
