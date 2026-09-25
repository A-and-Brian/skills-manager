//! `skills-manager-cli serve --stdio`: the app's host-scoped commands over a
//! pipe, so another machine's app can drive this one through a single `ssh`.
//!
//! Protocol 1, one JSON object per line in both directions:
//! - first, the server says hello: `{"hello":{"protocol":1,"version":…,"os":…,
//!   "arch":…,"home":…,"base_dir":…,"pid":…}}`;
//! - a request is `{"id":7,"cmd":"get_presets","args":{…}}`, `args` being the
//!   object the UI passes to `invoke`;
//! - its reply is `{"id":7,"ok":true,"result":…}` or `{"id":7,"ok":false,
//!   "error":{"kind":…,"message":…}}`. Each request runs on its own thread, so
//!   replies can arrive out of order; a line that is not a request is answered
//!   with `"id":null`;
//! - an event is `{"event":"install-progress","payload":…}`.
//!
//! Nothing else may reach stdout. The server exits when stdin closes.

use std::io::{self, BufRead, Write};
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::central_repo;
use super::error::AppError;
use super::file_watcher;
use super::host::{HostCtx, HostEvents};
use super::host_dispatch::dispatch;
use super::install_cancel::InstallCancelRegistry;
use super::skill_store::SkillStore;

pub const PROTOCOL: u32 = 1;

/// Who is answering: sent once, before anything else.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hello {
    pub protocol: u32,
    pub version: String,
    pub os: String,
    pub arch: String,
    pub home: String,
    pub base_dir: String,
    pub pid: u32,
}

#[derive(Serialize, Deserialize)]
pub struct HelloLine {
    pub hello: Hello,
}

#[derive(Deserialize)]
struct Request {
    id: u64,
    cmd: String,
    #[serde(default)]
    args: Value,
}

/// The server's output. Every message is written as one whole line under a
/// lock, so replies and events from different threads never interleave.
/// It is also the events sink commands report progress to.
#[derive(Clone)]
pub struct Wire(Arc<Mutex<Box<dyn Write + Send>>>);

impl Wire {
    pub fn new(writer: impl Write + Send + 'static) -> Self {
        Self(Arc::new(Mutex::new(Box::new(writer))))
    }

    fn send(&self, message: &Value) -> io::Result<()> {
        let mut line = serde_json::to_vec(message)?;
        line.push(b'\n');
        let mut out = self.0.lock().unwrap_or_else(|e| e.into_inner());
        out.write_all(&line)?;
        out.flush()
    }
}

impl HostEvents for Wire {
    fn emit(&self, event: &str, payload: Value) {
        if let Err(err) = self.send(&json!({ "event": event, "payload": payload })) {
            log::debug!("Failed to write event {event}: {err}");
        }
    }
}

fn hello() -> Hello {
    Hello {
        protocol: PROTOCOL,
        version: env!("CARGO_PKG_VERSION").to_string(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        home: dirs::home_dir()
            .map(|home| home.to_string_lossy().into_owned())
            .unwrap_or_default(),
        base_dir: central_repo::base_dir().to_string_lossy().into_owned(),
        pid: std::process::id(),
    }
}

/// Serve this process's stdin and stdout until stdin closes. Only the file
/// watcher runs alongside, so the other side hears about edits made here;
/// schedulers belong to this machine's own app.
pub fn serve_stdio(store: Arc<SkillStore>) -> io::Result<()> {
    let wire = Wire::new(io::stdout());
    let events: Arc<dyn HostEvents> = Arc::new(wire.clone());
    file_watcher::start_file_watcher(events.clone(), store.clone());
    let ctx = HostCtx {
        store,
        cancel: Arc::new(InstallCancelRegistry::new()),
        events,
    };
    run(io::stdin().lock(), wire, Arc::new(ctx))
}

/// Say hello, then answer requests from `reader` until it ends. Requests
/// still running at that point are finished and answered before returning.
pub fn run(mut reader: impl BufRead, wire: Wire, ctx: Arc<HostCtx>) -> io::Result<()> {
    wire.send(&json!({ "hello": hello() }))?;

    let mut running: Vec<JoinHandle<()>> = Vec::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let request: Request = match serde_json::from_slice(&line) {
            Ok(request) => request,
            Err(err) => {
                let error = AppError::invalid_input(format!("Malformed request: {err}"));
                wire.send(&json!({ "id": null, "ok": false, "error": error }))?;
                continue;
            }
        };
        running.retain(|handle| !handle.is_finished());
        let (wire, ctx) = (wire.clone(), ctx.clone());
        running.push(thread::spawn(move || answer(&wire, &ctx, request)));
    }

    for handle in running {
        let _ = handle.join();
    }
    Ok(())
}

fn answer(wire: &Wire, ctx: &HostCtx, request: Request) {
    // A panicking command must still be answered, or its caller waits forever.
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        dispatch(ctx, &request.cmd, &request.args)
    }))
    .unwrap_or_else(|_| Err(AppError::internal(format!("{} panicked", request.cmd))));
    let reply = match result {
        Ok(result) => json!({ "id": request.id, "ok": true, "result": result }),
        Err(error) => json!({ "id": request.id, "ok": false, "error": error }),
    };
    if let Err(err) = wire.send(&reply) {
        log::debug!("Failed to answer {}: {err}", request.cmd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::MutexGuard;

    /// An in-memory stdout the test can read back after `run` returns.
    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Vec<u8>>>);

    impl Write for Captured {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().write(buf)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct Served {
        lines: Vec<Value>,
        _tmp: tempfile::TempDir,
        _lock: MutexGuard<'static, ()>,
    }

    impl Drop for Served {
        fn drop(&mut self) {
            central_repo::set_test_base_dir_override(None);
        }
    }

    /// Run the server over `input` on a temp library, returning every line it
    /// wrote. Each line must be JSON: anything else would break the client.
    fn serve(input: &str) -> Served {
        let lock = central_repo::test_base_dir_lock();
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("repo");
        central_repo::set_test_base_dir_override(Some(base.clone()));
        std::fs::create_dir_all(central_repo::skills_dir()).unwrap();
        let store = SkillStore::new(&base.join("test.db")).unwrap();

        let out = Captured::default();
        let wire = Wire::new(out.clone());
        let ctx = HostCtx::for_tests(store, Arc::new(wire.clone()));
        run(Cursor::new(input.to_string()), wire, Arc::new(ctx)).unwrap();

        let text = String::from_utf8(out.0.lock().unwrap().clone()).unwrap();
        assert!(text.ends_with('\n'), "output ends mid-line: {text:?}");
        let lines = text
            .lines()
            .map(|line| {
                serde_json::from_str(line).unwrap_or_else(|e| panic!("not JSON ({e}): {line}"))
            })
            .collect();
        Served {
            lines,
            _tmp: tmp,
            _lock: lock,
        }
    }

    #[test]
    fn says_hello_first_even_with_no_requests() {
        let served = serve("");
        assert_eq!(served.lines.len(), 1);
        let hello: HelloLine = serde_json::from_value(served.lines[0].clone()).unwrap();
        assert_eq!(hello.hello.protocol, PROTOCOL);
        assert_eq!(hello.hello.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(hello.hello.pid, std::process::id());
        assert!(hello.hello.base_dir.ends_with("repo"));
    }

    #[test]
    fn answers_each_request_under_its_own_id() {
        let served = serve(concat!(
            r#"{"id":1,"cmd":"create_preset","args":{"name":"Work"}}"#,
            "\n",
            r#"{"id":2,"cmd":"get_tool_order_cmd","args":{}}"#,
            "\n",
            r#"{"id":3,"cmd":"git_backup_push"}"#,
            "\n",
        ));
        assert!(served.lines[0].get("hello").is_some());
        let reply = |id: u64| {
            served.lines[1..]
                .iter()
                .find(|line| line["id"] == id)
                .unwrap_or_else(|| panic!("no reply to {id}"))
        };
        assert_eq!(reply(1)["ok"], true);
        assert_eq!(reply(1)["result"]["name"], "Work");
        assert_eq!(reply(2)["ok"], true);
        assert!(reply(2)["result"].is_array());
        assert_eq!(reply(3)["ok"], false);
        assert_eq!(reply(3)["error"]["kind"], "not_found");
        assert_eq!(served.lines.len(), 4);
    }

    #[test]
    fn a_malformed_line_is_answered_and_the_stream_goes_on() {
        let served = serve(concat!(
            "not json\n",
            "\n",
            r#"{"cmd":"get_presets"}"#,
            "\n",
            r#"{"id":9,"cmd":"get_presets","args":{}}"#,
            "\n",
        ));
        let errors: Vec<&Value> = served.lines[1..]
            .iter()
            .filter(|line| line["id"].is_null())
            .collect();
        assert_eq!(errors.len(), 2, "{:?}", served.lines);
        for error in errors {
            assert_eq!(error["ok"], false);
            assert_eq!(error["error"]["kind"], "invalid_input");
        }
        let last = served.lines.last().unwrap();
        assert_eq!(
            (last["id"].as_u64(), last["ok"].as_bool()),
            (Some(9), Some(true))
        );
    }

    #[test]
    fn events_are_written_as_event_lines() {
        let out = Captured::default();
        let wire = Wire::new(out.clone());
        wire.emit("batch-import-progress", json!({ "current": 1, "total": 2 }));
        wire.emit("app-files-changed", Value::Null);

        let text = String::from_utf8(out.0.lock().unwrap().clone()).unwrap();
        let lines: Vec<Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(
            lines,
            vec![
                json!({ "event": "batch-import-progress", "payload": { "current": 1, "total": 2 } }),
                json!({ "event": "app-files-changed", "payload": null }),
            ]
        );
    }
}
