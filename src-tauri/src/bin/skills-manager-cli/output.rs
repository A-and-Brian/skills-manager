//! Printing reports, and the `--json` failure envelope.

use app_lib::core::error::AppError;
use serde::Serialize;

/// Keep the `AppError` itself in the chain rather than only its sentence:
/// `main` downcasts it to emit a machine-readable JSON envelope for the kinds
/// that carry details (a deployment refusal names the paths in the way).
/// The `--json` failure envelope.
///
/// Most failures are one sentence and `COMMAND_FAILED`. A few carry specifics
/// the caller has to act on rather than repeat — a deployment refusal names
/// the paths in the way — and those pass their structure straight through, so
/// an agent can say which directory is blocking and offer the way out (#363).
pub(crate) fn error_envelope(err: &anyhow::Error) -> serde_json::Value {
    let message = format!("{err:#}");
    match err.downcast_ref::<AppError>() {
        Some(app_err) if app_err.details.is_some() => {
            let mut value = serde_json::to_value(app_err).unwrap();
            let object = value.as_object_mut().unwrap();
            object.insert("ok".into(), serde_json::Value::Bool(false));
            // Derive the code from the kind that is already on the wire, so a
            // future detail-carrying kind cannot ship a contradicting code.
            let code = object
                .get("kind")
                .and_then(|kind| kind.as_str())
                .unwrap_or("command_failed")
                .to_ascii_uppercase();
            object.insert("code".into(), serde_json::Value::String(code));
            object.insert("error".into(), serde_json::Value::String(message));
            value
        }
        _ => serde_json::json!({
            "ok": false,
            "code": "COMMAND_FAILED",
            "message": message.clone(),
            "error": message,
        }),
    }
}

pub(crate) fn map_app_err(e: AppError) -> anyhow::Error {
    anyhow::Error::new(e)
}

pub(crate) fn print_json<T: Serialize>(value: &T, json: bool) {
    let rendered = if json {
        serde_json::to_string(value).unwrap()
    } else {
        serde_json::to_string_pretty(value).unwrap()
    };
    println!("{rendered}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    /// An agent has to name the directory that is in the way and say the
    /// contents survived. Flattening the refusal into one sentence is what
    /// made that impossible, so the paths must reach the envelope as data.
    #[test]
    fn a_deployment_refusal_keeps_its_paths_in_the_json_envelope() {
        let err = map_app_err(AppError::target_conflict(
            "Refusing to deploy: 1 of 2 target(s) …",
            vec![app_lib::core::error::TargetConflictDetail {
                path: "/home/me/.claude/skills/db".to_string(),
                reason: "is not a managed deployment".to_string(),
            }],
        ));

        let envelope = error_envelope(&err);
        assert_eq!(envelope["ok"], false);
        assert_eq!(envelope["code"], "TARGET_CONFLICT");
        assert_eq!(envelope["kind"], "target_conflict");
        assert_eq!(
            envelope["details"]["conflicts"][0]["path"],
            "/home/me/.claude/skills/db"
        );
    }

    /// Everything else keeps the shape callers already parse.
    #[test]
    fn an_ordinary_failure_keeps_the_command_failed_envelope() {
        let envelope = error_envelope(&anyhow!("no agent key provided"));
        assert_eq!(envelope["code"], "COMMAND_FAILED");
        assert_eq!(envelope["message"], "no agent key provided");
        assert!(envelope.get("details").is_none());
    }
}
