use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use persona_system::{
    ApplyNiriEvent, FocusObservation, FocusStatisticsProbe, FocusTracker, NiriEvent,
    NiriFocusSource, NiriWindowId, ReadFocusStatistics, SystemTarget,
};

struct SourceFile {
    path: PathBuf,
    content: String,
}

impl SourceFile {
    fn read(path: PathBuf) -> Self {
        let content = fs::read_to_string(&path).expect("source file is readable");
        Self { path, content }
    }

    fn is_guard_source(&self) -> bool {
        self.path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == "actor_runtime_truth.rs")
    }

    fn contains(&self, fragment: &str) -> bool {
        self.content.contains(fragment)
    }
}

struct SourceTree {
    root: PathBuf,
}

impl SourceTree {
    fn new() -> Self {
        Self {
            root: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        }
    }

    fn guarded_files(&self) -> Vec<SourceFile> {
        let mut files = vec![self.root.join("Cargo.toml"), self.root.join("Cargo.lock")];
        files.extend(self.source_files());
        files.extend(self.test_files());
        files.into_iter().map(SourceFile::read).collect()
    }

    fn source_files(&self) -> Vec<PathBuf> {
        let src = self.root.join("src");
        fs::read_dir(src)
            .expect("source directory is readable")
            .map(|entry| entry.expect("source entry is readable").path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "rs"))
            .collect()
    }

    fn test_files(&self) -> Vec<PathBuf> {
        let tests = self.root.join("tests");
        fs::read_dir(tests)
            .expect("tests directory is readable")
            .map(|entry| entry.expect("test entry is readable").path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "rs"))
            .collect()
    }
}

#[test]
fn niri_focus_cannot_use_non_kameo_runtime() {
    let forbidden_fragments = [
        "ractor =",
        "name = \"ractor\"",
        "use ractor",
        "ractor::",
        "RpcReplyPort",
        "ActorProcessingErr",
    ];

    let mut violations = Vec::new();
    for file in SourceTree::new().guarded_files() {
        if file.is_guard_source() {
            continue;
        }
        for fragment in forbidden_fragments {
            if file.contains(fragment) {
                violations.push(format!("{} contains {fragment}", file.path.display()));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "non-kameo niri actor runtime violations:\n{}",
        violations.join("\n")
    );
}

#[test]
fn niri_subscription_cannot_bypass_focus_actor_mailbox() {
    let source = SourceFile::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("niri.rs"),
    );

    assert!(source.contains("FocusTracker::start"));
    assert!(source.contains("focus.ask(ApplyNiriEvent { event }).send()"));
    assert!(!source.contains("tracker.apply_event(&event)"));
}

#[test]
fn focus_tracker_actor_cannot_be_empty_marker() {
    let source = SourceFile::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("niri.rs"),
    );

    assert!(source.contains("pub struct FocusTracker {"));
    assert!(source.contains("last: Option<FocusObservation>,"));
    assert!(source.contains("generations: HashMap<u64, u64>,"));
    assert!(source.contains("applied_event_count: u64,"));
    assert!(source.contains("emitted_observation_count: u64,"));
}

#[tokio::test]
async fn niri_focus_cannot_emit_target_chatter_without_focus_change() {
    let target = SystemTarget::niri_window(10);
    let mut tracker = FocusTracker::new(target, NiriWindowId::new(10));
    tracker.accept(FocusObservation::new(target, false, 1_000_000_005));
    let focus = FocusTracker::start(tracker).await;

    let observations = focus
        .ask(ApplyNiriEvent {
            event: window_event(false, 1, 5, "Responder spinner"),
        })
        .await
        .expect("actor applies niri event");

    assert!(observations.is_empty());
    FocusTracker::stop(focus).await.expect("actor stops");
}

#[tokio::test]
async fn niri_focus_cannot_forget_previous_window_observation_between_messages() {
    let target = SystemTarget::niri_window(10);
    let focus = FocusTracker::start(FocusTracker::new(target, NiriWindowId::new(10))).await;

    let first = focus
        .ask(ApplyNiriEvent {
            event: window_event(false, 1, 5, "Responder"),
        })
        .await
        .expect("actor applies first event");
    let repeated = focus
        .ask(ApplyNiriEvent {
            event: window_event(false, 1, 5, "Responder renamed"),
        })
        .await
        .expect("actor applies repeated event");

    assert_eq!(
        first,
        vec![FocusObservation::new(target, false, 1_000_000_005)]
    );
    assert!(repeated.is_empty());
    let statistics = focus
        .ask(ReadFocusStatistics {
            probe: FocusStatisticsProbe::expecting_at_least(2, 1),
        })
        .await
        .expect("actor statistics read through typed message");

    assert_eq!(statistics.applied_event_count(), 2);
    assert_eq!(statistics.emitted_observation_count(), 1);
    assert!(statistics.satisfied());

    FocusTracker::stop(focus).await.expect("actor stops");
}

#[tokio::test]
async fn niri_focus_cannot_leak_state_between_concurrent_subscribers() {
    let target = SystemTarget::niri_window(10);
    let primary = FocusTracker::start(FocusTracker::new(target, NiriWindowId::new(10))).await;
    let secondary = FocusTracker::start(FocusTracker::new(target, NiriWindowId::new(10))).await;

    let focused_event = window_event(true, 1, 5, "Responder");
    let primary_first = primary
        .ask(ApplyNiriEvent {
            event: focused_event.clone(),
        })
        .await
        .expect("primary actor applies first event");
    let secondary_first = secondary
        .ask(ApplyNiriEvent {
            event: focused_event.clone(),
        })
        .await
        .expect("secondary actor applies first event");

    let expected_first = vec![FocusObservation::new(target, true, 1_000_000_005)];
    assert_eq!(primary_first, expected_first);
    assert_eq!(secondary_first, expected_first);

    let duplicate_event = window_event(true, 1, 5, "Responder renamed");
    let primary_duplicate = primary
        .ask(ApplyNiriEvent {
            event: duplicate_event.clone(),
        })
        .await
        .expect("primary actor applies duplicate event");
    let secondary_duplicate = secondary
        .ask(ApplyNiriEvent {
            event: duplicate_event,
        })
        .await
        .expect("secondary actor applies duplicate event");

    assert!(primary_duplicate.is_empty());
    assert!(secondary_duplicate.is_empty());

    let unfocused_event = window_event(false, 2, 9, "Responder");
    let primary_change = primary
        .ask(ApplyNiriEvent {
            event: unfocused_event.clone(),
        })
        .await
        .expect("primary actor applies focus-change event");
    let secondary_change = secondary
        .ask(ApplyNiriEvent {
            event: unfocused_event,
        })
        .await
        .expect("secondary actor applies focus-change event");

    let expected_change = vec![FocusObservation::new(target, false, 2_000_000_009)];
    assert_eq!(primary_change, expected_change);
    assert_eq!(secondary_change, expected_change);

    let primary_statistics = primary
        .ask(ReadFocusStatistics {
            probe: FocusStatisticsProbe::expecting_at_least(3, 2),
        })
        .await
        .expect("primary statistics read through typed message");
    let secondary_statistics = secondary
        .ask(ReadFocusStatistics {
            probe: FocusStatisticsProbe::expecting_at_least(3, 2),
        })
        .await
        .expect("secondary statistics read through typed message");

    assert_eq!(primary_statistics.applied_event_count(), 3);
    assert_eq!(primary_statistics.emitted_observation_count(), 2);
    assert!(primary_statistics.satisfied());
    assert_eq!(secondary_statistics.applied_event_count(), 3);
    assert_eq!(secondary_statistics.emitted_observation_count(), 2);
    assert!(secondary_statistics.satisfied());

    FocusTracker::stop(primary).await.expect("primary stops");
    FocusTracker::stop(secondary)
        .await
        .expect("secondary stops");
}

#[test]
fn niri_subscription_cannot_poll_focus_snapshots() {
    let fixture = FakeNiri::new("niri-subscription-cannot-poll-focus-snapshots");
    let source = NiriFocusSource::with_command(fixture.command());
    let mut output = Vec::new();

    source
        .subscribe(SystemTarget::niri_window(10), &mut output)
        .expect("fake niri event stream drives subscription");
    let text = String::from_utf8(output).expect("subscription output is utf8");
    let lines = text.lines().collect::<Vec<_>>();

    assert_eq!(
        lines,
        vec![
            "(FocusObservation ((NiriWindow 10) False 1000000005))",
            "(FocusObservation ((NiriWindow 10) True 2000000007))",
        ]
    );

    let source_file = SourceFile::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("niri.rs"),
    );
    assert!(source_file.contains("\"event-stream\""));
    assert!(source_file.contains("focus.ask(ApplyNiriEvent { event }).send()"));
    assert!(!source_file.contains("tracker.apply_event(&event)"));
}

fn window_event(focused: bool, seconds: u64, nanos: u32, title: &str) -> NiriEvent {
    NiriEvent::from_json_str(&format!(
        r#"{{
          "WindowOpenedOrChanged": {{
            "window": {{
              "id": 10,
              "title": "{title}",
              "app_id": "org.wezfurlong.wezterm",
              "pid": 100,
              "workspace_id": 1,
              "is_focused": {focused},
              "is_floating": false,
              "is_urgent": false,
              "layout": {{}},
              "focus_timestamp": {{"secs": {seconds}, "nanos": {nanos}}}
            }}
          }}
        }}"#
    ))
    .expect("event decodes")
}

struct FakeNiri {
    root: PathBuf,
    command: PathBuf,
}

impl FakeNiri {
    fn new(name: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "persona-system-{name}-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("fake niri directory exists");
        let windows = root.join("windows.json");
        let events = root.join("events.jsonl");
        let command = root.join("niri");

        fs::write(&windows, Self::windows()).expect("fake windows fixture writes");
        fs::write(&events, Self::events()).expect("fake event stream fixture writes");
        fs::write(
            &command,
            format!(
                r#"#!/bin/sh
set -eu
if [ "$1" = "msg" ] && [ "$2" = "--json" ] && [ "$3" = "windows" ]; then
  cat {}
elif [ "$1" = "msg" ] && [ "$2" = "--json" ] && [ "$3" = "event-stream" ]; then
  cat {}
else
  echo "unexpected fake niri arguments: $*" >&2
  exit 64
fi
"#,
                shell_quote(&windows),
                shell_quote(&events),
            ),
        )
        .expect("fake niri command writes");
        let mut permissions = fs::metadata(&command)
            .expect("fake niri metadata reads")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&command, permissions).expect("fake niri is executable");

        Self { root, command }
    }

    fn command(&self) -> String {
        self.command.to_string_lossy().to_string()
    }

    fn windows() -> &'static str {
        r#"[
  {
    "id": 10,
    "title": "Responder",
    "app_id": "com.ligoldragon.personasystemfakeniri",
    "pid": 100,
    "workspace_id": 1,
    "is_focused": false,
    "is_floating": false,
    "is_urgent": false,
    "layout": {},
    "focus_timestamp": {"secs": 1, "nanos": 5}
  }
]
"#
    }

    fn events() -> &'static str {
        r#"{"WindowOpenedOrChanged":{"window":{"id":10,"title":"Responder","app_id":"com.ligoldragon.personasystemfakeniri","pid":100,"workspace_id":1,"is_focused":true,"is_floating":false,"is_urgent":false,"layout":{},"focus_timestamp":{"secs":2,"nanos":7}}}}
{"WindowOpenedOrChanged":{"window":{"id":11,"title":"Other","app_id":"com.ligoldragon.personasystemfakeniri","pid":101,"workspace_id":1,"is_focused":true,"is_floating":false,"is_urgent":false,"layout":{},"focus_timestamp":{"secs":3,"nanos":9}}}}
"#
    }
}

impl Drop for FakeNiri {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}
