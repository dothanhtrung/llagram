use llagram::prompts::Prompts;
use std::collections::HashSet;
use std::fs;

#[test]
fn missing_prompt_reads_as_empty() {
    let dir = tempfile::tempdir().unwrap();
    let prompts = Prompts::new(dir.path().to_path_buf());
    assert!(prompts.read("NO_SUCH_FILE").is_empty());
    assert_eq!(prompts.dir(), dir.path());
}

#[test]
fn render_replaces_placeholders() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("MEMORY_NEW.md"),
        "Transcript:\n{transcript}\n",
    )
    .unwrap();
    let prompts = Prompts::new(dir.path().to_path_buf());
    let rendered = prompts.render("MEMORY_NEW", &[("transcript", "user: hi")]);
    assert_eq!(rendered, "Transcript:\nuser: hi");
}

#[test]
fn system_message_joins_global_personality_sorted_skills_and_memory() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("GLOBAL_INSTRUCTS.md"), "global").unwrap();
    fs::write(dir.path().join("PERSONALITY.md"), "persona").unwrap();
    fs::write(dir.path().join("MEMORY.md"), "mem:{memory}").unwrap();
    fs::write(dir.path().join("SKILL_WEB.md"), "web-skill").unwrap();
    fs::write(dir.path().join("SKILL_OTHER.md"), "other-skill").unwrap();
    let prompts = Prompts::new(dir.path().to_path_buf());

    let mut enabled = HashSet::new();
    enabled.insert("web".into());
    enabled.insert("other".into());
    let system = prompts.system_message("fact", &enabled);
    assert_eq!(
        system,
        "global\n\npersona\n\nother-skill\n\nweb-skill\n\nmem:fact"
    );

    let empty = prompts.system_message("   ", &HashSet::new());
    assert_eq!(empty, "global\n\npersona");
}

#[test]
fn repo_prompt_files_are_present() {
    let prompts = Prompts::new(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("prompts"),
    );
    assert!(!prompts.read("GLOBAL_INSTRUCTS").is_empty());
    assert!(!prompts.read("PERSONALITY").is_empty());
    assert!(prompts.read("MEMORY").contains("{memory}"));
    assert!(prompts.read("MEMORY_NEW").contains("{transcript}"));
    assert!(prompts.read("MEMORY_UPDATE").contains("{existing}"));
    assert!(prompts.read("MEMORY_COMPRESS").contains("{max_size}"));
    assert!(!prompts.read("SKILL_WEB").is_empty());
}
