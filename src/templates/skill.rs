//! The `flits-video` skill for Claude Code (and other coding agents that read `SKILL.md` skills):
//! how to make a video with flits (the render-and-look loop), motion design craft, gotchas and an API
//! cheat sheet. `flits init` puts it in the project; `flits skill install` in a project or globally.

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};

/// Folder name of the skill (also its `name` in the SKILL.md front matter).
pub const NAME: &str = "flits-video";

/// The skill's files, relative to its folder.
pub const FILES: &[(&str, &str)] = &[
    ("SKILL.md", include_str!("skill/SKILL.md")),
    ("api.md", include_str!("skill/api.md")),
    ("example.ts", include_str!("skill/example.ts")),
];

/// `<root>/.claude/skills/flits-video`
pub fn project_dir(root: &Path) -> PathBuf {
    root.join(".claude").join("skills").join(NAME)
}

/// `~/.claude/skills/flits-video` (`$HOME`, else `%USERPROFILE%`).
pub fn global_dir() -> Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty())
        .ok_or_else(|| anyhow!("can't find your home folder (set HOME); or install into a project without --global"))?;
    Ok(project_dir(Path::new(&home)))
}

/// Write the skill into `dir` (created; files of an older version are replaced). Returns the files written.
pub fn install(dir: &Path) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    FILES
        .iter()
        .map(|(name, content)| {
            let p = dir.join(name);
            std::fs::write(&p, content).with_context(|| format!("writing {}", p.display()))?;
            Ok(p)
        })
        .collect()
}

/// All files as one text (for `flits skill print`): SKILL.md first, the others after a header line.
pub fn print() -> String {
    let mut out = String::new();
    for (i, (name, content)) in FILES.iter().enumerate() {
        if i > 0 {
            out.push_str(&format!("\n\n===== {name} =====\n\n"));
        }
        out.push_str(content);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_has_front_matter_and_installs() {
        let skill = &FILES[0].1.replace("\r\n", "\n");
        let front = skill.strip_prefix("---\n").and_then(|s| s.split_once("\n---\n")).expect("YAML front matter").0;
        assert!(front.lines().any(|l| l == format!("name: {NAME}")), "{front}");
        let desc = front.lines().find_map(|l| l.strip_prefix("description: ")).expect("description");
        for word in ["video", "render", "captions", "motion graphics"] {
            assert!(desc.contains(word), "description should mention {word}");
        }
        // every file the skill refers to ships with it
        for (name, _) in &FILES[1..] {
            assert!(skill.contains(&format!("`{name}`")), "SKILL.md should point to {name}");
        }
        assert!(skill.contains("flits-motion-library: start"), "placeholder for the motion library");

        let dir = std::env::temp_dir().join(format!("flits-skill-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let files = install(&project_dir(&dir)).unwrap();
        assert_eq!(files.len(), FILES.len());
        assert!(dir.join(".claude/skills/flits-video/SKILL.md").is_file());
        install(&project_dir(&dir)).unwrap(); // again: replaces
        assert!(print().contains("===== api.md ====="));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
