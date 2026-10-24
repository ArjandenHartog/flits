//! The CLI as a coding agent uses it: `check --json`, stills and sheets whose output paths and
//! labels can be read back, and `skill install`/`print`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn flits() -> Command {
    Command::new(env!("CARGO_BIN_EXE_flits"))
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join("flits-tests").join(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn ok(o: Output) -> String {
    assert!(o.status.success(), "stdout: {}\nstderr: {}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    String::from_utf8(o.stdout).unwrap()
}

fn scene(d: &Path) -> PathBuf {
    let p = d.join("scene.json");
    let s = serde_json::json!({
        "width": 640, "height": 360, "fps": 10, "duration": 3, "background": "#101014",
        "layers": [
            { "type": "text", "text": "Fits nicely", "size": 40, "y": 120 },
            { "type": "text", "text": "This headline is far too wide", "size": 60, "x": 600, "start": 1 },
            { "type": "group", "id": "card", "children": [{ "type": "rect", "width": 100, "height": 50, "color": "#f26328" }] }
        ]
    });
    std::fs::write(&p, s.to_string()).unwrap();
    p
}

#[test]
fn check_json_reports_layout_issues_and_the_timeline() {
    let d = dir("agent-check");
    let out = ok(flits().arg("check").arg("--json").arg(scene(&d)).output().unwrap());
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!((v["width"].as_u64(), v["duration"].as_f64(), v["frames"].as_i64()), (Some(640), Some(3.0), Some(30)));
    let issues = v["issues"].as_array().unwrap();
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert_eq!(issues[0]["kind"], "offscreen");
    assert_eq!(issues[0]["layer"], "This headline is far too wide");
    assert_eq!(issues[0]["from"].as_f64(), Some(1.0));
    let layers = v["layers"].as_array().unwrap();
    assert_eq!(layers[0]["type"], "text");
    assert_eq!(layers[2]["label"], "card");
    assert_eq!(layers[2]["children"][0]["type"], "rect");

    // the plain output carries the same issue as a warning (what the TS/Python wrappers collect)
    let plain = ok(flits().arg("check").arg(scene(&d)).output().unwrap());
    assert!(plain.lines().any(|l| l.starts_with("warning: 1.00-3.00s: text \"This headline")), "{plain}");

    // a broken scene: still one JSON object, and a failing exit code
    let bad = d.join("bad.json");
    std::fs::write(
        &bad,
        r#"{"width": 640, "height": 360, "duration": 1, "layers": [{"type": "text", "text": "x", "x": [[0, 0], [1, 5, "expo-outt"]]}]}"#,
    )
    .unwrap();
    let o = flits().arg("check").arg("--json").arg(&bad).output().unwrap();
    assert!(!o.status.success());
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["ok"], false);
    assert!(v["error"].as_str().unwrap().contains("unknown ease 'expo-outt'"), "{v}");
}

#[test]
fn stills_and_sheets_print_their_paths() {
    let d = dir("agent-sheet");
    let s = scene(&d);
    let out = ok(flits().arg("still").arg(&s).args(["--at", "0.5", "--at", "2", "-o"]).arg(d.join("s.png")).output().unwrap());
    let paths: Vec<&str> = out.lines().collect();
    assert_eq!(paths.len(), 2, "{out}");
    assert!(paths[0].ends_with("s-0.50s.png") && paths[1].ends_with("s-2.00s.png"), "{out}");
    assert!(paths.iter().all(|p| Path::new(p).is_file()));

    // exact times: 3 tiles in one row
    let sheet = d.join("sheet.png");
    let out = ok(flits()
        .arg("sheet")
        .arg(&s)
        .args(["--at", "0.5", "--at", "1", "--at", "2.5", "--thumb", "160", "-o"])
        .arg(&sheet)
        .output()
        .unwrap());
    assert_eq!(out.trim(), sheet.display().to_string());
    let png = std::fs::read(&sheet).unwrap();
    let (w, h) = (u32::from_be_bytes(png[16..20].try_into().unwrap()), u32::from_be_bytes(png[20..24].try_into().unwrap()));
    assert_eq!(w, 3 * (160 + 8) + 8);
    assert_eq!(h, 90 + 34 + 2 * 8);

    // a range: --count frames from --from to --to, 4 per row
    ok(flits()
        .arg("sheet")
        .arg(&s)
        .args(["--from", "1", "--to", "2", "--count", "6", "--thumb", "160", "-o"])
        .arg(&sheet)
        .output()
        .unwrap());
    let png = std::fs::read(&sheet).unwrap();
    assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), 2 * (90 + 34 + 8) + 8);
}

#[test]
fn skill_install_and_print() {
    let d = dir("agent-skill");
    let out = ok(flits().args(["skill", "install", "--dir"]).arg(&d).output().unwrap());
    assert_eq!(out.lines().count(), 3, "{out}");
    let skill = std::fs::read_to_string(d.join(".claude/skills/flits-video/SKILL.md")).unwrap();
    assert!(skill.contains("name: flits-video"));

    let home = d.join("home");
    std::fs::create_dir_all(&home).unwrap();
    ok(flits().args(["skill", "install", "--global"]).env("HOME", &home).env("USERPROFILE", &home).output().unwrap());
    assert!(home.join(".claude/skills/flits-video/api.md").is_file());

    let printed = ok(flits().args(["skill", "print"]).output().unwrap());
    assert!(printed.starts_with("---\nname: flits-video") && printed.contains("===== example.ts ====="));
}
