//! `flits init`: a TypeScript video project (package.json, tsconfig, an example video, a component,
//! a JSX example) with the flits SDK vendored in `flits/` and imported as `#flits`.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

/// The SDK files (bindings/typescript), written to `<project>/flits/`.
pub const SDK: &[(&str, &str)] = &[
    ("index.js", include_str!("../../bindings/typescript/index.js")),
    ("index.d.ts", include_str!("../../bindings/typescript/index.d.ts")),
    ("scene.d.ts", include_str!("../../bindings/typescript/scene.d.ts")),
    ("compile.js", include_str!("../../bindings/typescript/compile.js")),
    ("compile.d.ts", include_str!("../../bindings/typescript/compile.d.ts")),
    ("components.js", include_str!("../../bindings/typescript/components.js")),
    ("components.d.ts", include_str!("../../bindings/typescript/components.d.ts")),
    ("jsx-runtime.js", include_str!("../../bindings/typescript/jsx-runtime.js")),
    ("jsx-runtime.d.ts", include_str!("../../bindings/typescript/jsx-runtime.d.ts")),
    ("jsx-dev-runtime.js", include_str!("../../bindings/typescript/jsx-dev-runtime.js")),
    ("jsx-dev-runtime.d.ts", include_str!("../../bindings/typescript/jsx-dev-runtime.d.ts")),
];

const FILES: &[(&str, &str)] = &[
    ("package.json", include_str!("project/package.json")),
    ("tsconfig.json", include_str!("project/tsconfig.json")),
    ("README.md", include_str!("project/README.md")),
    (".gitignore", include_str!("project/gitignore")),
    ("src/video.ts", include_str!("project/video.ts")),
    ("src/components.ts", include_str!("project/components.ts")),
    ("src/data.json", include_str!("project/data.json")),
    ("src/hello.tsx", include_str!("project/hello.tsx")),
];

/// Write the SDK into `<dir>/flits/` (replacing an older copy). Returns the files written.
pub fn write_sdk(dir: &Path) -> Result<Vec<PathBuf>> {
    let sdk = dir.join("flits");
    std::fs::create_dir_all(&sdk).with_context(|| format!("creating {}", sdk.display()))?;
    let mut out = Vec::new();
    for (name, content) in SDK {
        let p = sdk.join(name);
        std::fs::write(&p, content).with_context(|| format!("writing {}", p.display()))?;
        out.push(p);
    }
    std::fs::write(sdk.join("VERSION"), format!("{}\n", env!("CARGO_PKG_VERSION")))?;
    Ok(out)
}

/// Create a new project in `dir` (which must not exist or be empty).
pub fn init(dir: &Path) -> Result<Vec<PathBuf>> {
    if dir.exists() && std::fs::read_dir(dir)?.next().is_some() {
        bail!("{} is not empty (use `flits init --update` inside a project to refresh its flits/ SDK)", dir.display());
    }
    // package name: the folder name, lowercase with dashes ("." -> the current folder's name)
    let name = std::fs::canonicalize(dir)
        .ok()
        .and_then(|d| d.file_name().map(|n| n.to_os_string()))
        .or_else(|| dir.file_name().map(|n| n.to_os_string()))
        .map(|n| n.to_string_lossy().to_lowercase().replace(|c: char| !c.is_ascii_alphanumeric() && c != '-', "-"))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "flits-video".into());
    let mut written = Vec::new();
    for (rel, content) in FILES {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap())?;
        std::fs::write(&p, content.replace("{{name}}", &name))?;
        written.push(p);
    }
    written.extend(write_sdk(dir)?);
    Ok(written)
}

/// The nearest folder (from `start` up) whose package.json maps `#flits` (a flits project).
pub fn project_root(start: &Path) -> Option<PathBuf> {
    let start = std::fs::canonicalize(start).ok()?;
    start
        .ancestors()
        .find(|d| std::fs::read_to_string(d.join("package.json")).is_ok_and(|s| s.contains("\"#flits\"")))
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_writes_a_project_with_the_sdk() {
        let dir = std::env::temp_dir().join(format!("flits-init-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let files = init(&dir.join("My Video")).unwrap();
        let root = dir.join("My Video");
        assert!(files.contains(&root.join("src/video.ts")) && root.join("flits/components.js").is_file());
        assert!(std::fs::read_to_string(root.join("package.json")).unwrap().contains("\"name\": \"my-video\""));
        assert_eq!(project_root(&root.join("src")).unwrap(), std::fs::canonicalize(&root).unwrap());
        assert!(init(&root).is_err(), "refuses a non-empty folder");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
