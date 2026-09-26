//! The `--json` reports agents parse, as `skills/manage-skills/SKILL.md`
//! documents them. These pin the shape — top-level keys, value types, and the
//! stable `code` on failures — not the values, so they survive data changes.
//! Each test gets its own temp base dir and temp HOME, so nothing here reads
//! or writes this machine's library or agent folders. Unix only, like
//! `serve_stdio.rs`: on Windows the home folder does not come from HOME.
#![cfg(unix)]

use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

const CLI: &str = env!("CARGO_BIN_EXE_skills-manager-cli");

struct Sandbox(TempDir);

impl Sandbox {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("home")).unwrap();
        Sandbox(tmp)
    }

    /// A library holding one local skill, `demo`, tagged `web` and a member
    /// of the preset `Web`, set up through the CLI's own commands.
    fn seeded() -> Self {
        let sandbox = Self::new();
        let source = sandbox.0.path().join("source/demo");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(
            source.join("SKILL.md"),
            "---\nname: demo\ndescription: A demo skill\n---\nbody\n",
        )
        .unwrap();
        sandbox.ok(&["skills", "install", source.to_str().unwrap()]);
        sandbox.ok(&["presets", "create", "Web"]);
        sandbox.ok(&["presets", "add-skill", "Web", "demo"]);
        sandbox.ok(&["skills", "tag", "add", "demo", "web"]);
        sandbox
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(CLI)
            .arg("--json")
            .arg("--base-dir")
            .arg(self.0.path().join("base"))
            .args(args)
            .env("HOME", self.0.path().join("home"))
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("XDG_DATA_HOME")
            .output()
            .unwrap()
    }

    /// The report on stdout of a command that must succeed.
    fn ok(&self, args: &[&str]) -> Value {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "{args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    }

    /// The exit code and the envelope on stderr of a command that must fail.
    fn fail(&self, args: &[&str]) -> (i32, Value) {
        let out = self.run(args);
        assert!(!out.status.success(), "{args:?} unexpectedly succeeded");
        (
            out.status.code().unwrap(),
            serde_json::from_slice(&out.stderr).unwrap(),
        )
    }
}

/// Assert `value` is an object carrying each key with a value of the given
/// JSON type: "string", "bool", "number", "array", "object", or "string?"
/// for a string that may be null.
fn assert_shape(value: &Value, fields: &[(&str, &str)]) {
    let object = value
        .as_object()
        .unwrap_or_else(|| panic!("not an object: {value}"));
    for (key, kind) in fields {
        let field = object
            .get(*key)
            .unwrap_or_else(|| panic!("missing `{key}` in {value}"));
        let matches = match *kind {
            "string" => field.is_string(),
            "string?" => field.is_string() || field.is_null(),
            "bool" => field.is_boolean(),
            "number" => field.is_number(),
            "array" => field.is_array(),
            "object" => field.is_object(),
            other => panic!("unknown kind {other}"),
        };
        assert!(matches, "`{key}` is not {kind} in {value}");
    }
}

const SKILL_FIELDS: &[(&str, &str)] = &[
    ("id", "string"),
    ("name", "string"),
    ("description", "string?"),
    ("path", "string"),
    ("enabled", "bool"),
    ("tags", "array"),
    ("source_type", "string"),
    ("source_ref", "string?"),
    ("preset_ids", "array"),
    ("presets", "array"),
    ("deployed_to", "array"),
];

const PRESET_FIELDS: &[(&str, &str)] = &[
    ("id", "string"),
    ("name", "string"),
    ("description", "string?"),
    ("skill_count", "number"),
    ("active", "bool"),
];

#[test]
fn skills_list_reports_each_skill_and_filters() {
    let sandbox = Sandbox::seeded();

    let all = sandbox.ok(&["skills", "list"]);
    let skills = all.as_array().unwrap();
    assert_eq!(skills.len(), 1);
    assert_shape(&skills[0], SKILL_FIELDS);
    assert_eq!(skills[0]["presets"][0], "Web");
    assert_eq!(skills[0]["tags"][0], "web");

    let tagged = sandbox.ok(&["skills", "list", "--tag", "web"]);
    assert_eq!(tagged.as_array().unwrap().len(), 1);
    let other = sandbox.ok(&["skills", "list", "--tag", "other"]);
    assert_eq!(other, Value::Array(vec![]));
}

#[test]
fn skills_status_adds_per_agent_deployment_rows() {
    let sandbox = Sandbox::seeded();

    let status = sandbox.ok(&["skills", "status", "demo"]);
    assert_shape(&status, SKILL_FIELDS);
    assert_shape(&status, &[("agents", "array")]);
    let agents = status["agents"].as_array().unwrap();
    assert!(!agents.is_empty());
    assert_shape(
        &agents[0],
        &[
            ("key", "string"),
            ("display_name", "string"),
            ("installed", "bool"),
            ("globally_enabled", "bool"),
            ("deployed", "bool"),
            ("target_path", "string?"),
        ],
    );
}

#[test]
fn presets_status_reports_the_preset_and_its_agents() {
    let sandbox = Sandbox::seeded();

    let status = sandbox.ok(&["presets", "status", "Web", "--agent", "claude_code"]);
    assert_shape(&status, &[("preset", "object"), ("agents", "array")]);
    assert_shape(&status["preset"], PRESET_FIELDS);
    assert_shape(
        &status["agents"][0],
        &[
            ("key", "string"),
            ("display_name", "string"),
            ("deployed", "number"),
            ("total", "number"),
            ("status", "string"),
        ],
    );
}

#[test]
fn repo_status_and_agents_list_answer_the_health_check() {
    let sandbox = Sandbox::seeded();

    let repo = sandbox.ok(&["repo", "status"]);
    assert_shape(
        &repo,
        &[
            ("base_dir", "string"),
            ("skills_dir", "string"),
            ("db_path", "string"),
            ("skill_count", "number"),
            ("preset_count", "number"),
            ("active_preset_id", "string?"),
        ],
    );

    let agents = sandbox.ok(&["agents", "list"]);
    let agents = agents.as_array().unwrap();
    assert!(!agents.is_empty());
    assert_shape(
        &agents[0],
        &[
            ("key", "string"),
            ("display_name", "string"),
            ("installed", "bool"),
            ("skills_dir", "string"),
            ("enabled", "bool"),
            ("is_custom", "bool"),
        ],
    );
}

#[test]
fn failures_carry_a_stable_code_on_stderr() {
    let sandbox = Sandbox::new();
    let envelope = [
        ("ok", "bool"),
        ("code", "string"),
        ("message", "string"),
        ("error", "string"),
    ];

    let (exit, missing) = sandbox.fail(&["skills", "status", "no-such-skill"]);
    assert_eq!(exit, 1);
    assert_shape(&missing, &envelope);
    assert_eq!(missing["ok"], false);
    assert_eq!(missing["code"], "COMMAND_FAILED");

    let (exit, unknown) = sandbox.fail(&["no-such-command"]);
    assert_eq!(exit, 2);
    assert_shape(&unknown, &envelope);
    assert_eq!(unknown["ok"], false);
    assert_eq!(unknown["code"], "INVALID_ARGUMENT");
}
