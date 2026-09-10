use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Markdown prompts loaded from disk on each read so they can be edited
/// without rebuilding (or restarting) the bot.
#[derive(Debug, Clone)]
pub struct Prompts {
    dir: PathBuf,
}

impl Prompts {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Read `NAME.md`. Missing or unreadable files become empty.
    pub fn read(&self, stem: &str) -> String {
        let path = self.dir.join(format!("{stem}.md"));
        match fs::read_to_string(&path) {
            Ok(text) => text.trim().to_string(),
            Err(err) => {
                tracing::debug!("prompt {} not loaded: {err}", path.display());
                String::new()
            }
        }
    }

    pub fn render(&self, stem: &str, vars: &[(&str, &str)]) -> String {
        let mut text = self.read(stem);
        for (key, value) in vars {
            text = text.replace(&format!("{{{key}}}"), value);
        }
        text
    }

    /// One leading system message: global + personality + enabled skills + memory.
    /// Kept as a single system block so chat templates that require system-first stay valid.
    pub fn system_message(&self, memory: &str, enabled_skills: &HashSet<String>) -> String {
        let mut parts = Vec::new();
        push_nonempty(&mut parts, self.read("GLOBAL_INSTRUCTS"));
        push_nonempty(&mut parts, self.read("PERSONALITY"));
        let mut skills: Vec<_> = enabled_skills.iter().cloned().collect();
        skills.sort();
        for name in skills {
            push_nonempty(&mut parts, self.read(&format!("SKILL_{}", name.to_ascii_uppercase())));
        }
        if !memory.trim().is_empty() {
            push_nonempty(&mut parts, self.render("MEMORY", &[("memory", memory)]));
        }
        parts.join("\n\n")
    }
}

fn push_nonempty(parts: &mut Vec<String>, text: String) {
    if !text.is_empty() {
        parts.push(text);
    }
}
