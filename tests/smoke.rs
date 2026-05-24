use nota_codec::{Decoder, NotaDecode};
use system::{
    FocusObservation, FocusSubscription, FocusTracker, HarnessTarget, NiriEvent, NiriWindowId,
    NiriWindows, SystemRequest, SystemTarget,
};

#[test]
fn focus_observation_protects_owned_window() {
    let target = SystemTarget::niri_window(42);
    let harness = HarnessTarget::new("responder", target);
    let observation = FocusObservation::new(target, true, 7);

    assert!(observation.focused);
    assert!(harness.owns_target(observation.target));
}

#[test]
fn system_input_uses_noun_form_focus_subscription() {
    let mut decoder = Decoder::new("(FocusSubscription ((NiriWindow 198)))");
    let request = SystemRequest::decode(&mut decoder).expect("contract focus subscription decodes");

    let SystemRequest::FocusSubscription(FocusSubscription { target }) = request else {
        panic!("decoded input should be FocusSubscription");
    };
    assert_eq!(target, SystemTarget::niri_window(198));
}

#[test]
fn system_boundary_cannot_own_terminal_prompt_gate_records() {
    let scan = DriftScan::new(env!("CARGO_MANIFEST_DIR"));

    scan.assert_absent(&[
        "InputBuffer",
        "input-buffer",
        "prompt/input-buffer",
        "prompt-buffer",
        "accepts_injection",
    ]);
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DriftScan {
    root: std::path::PathBuf,
}

impl DriftScan {
    fn new(root: impl Into<std::path::PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn assert_absent(&self, forbidden_fragments: &[&str]) {
        let mut violations = Vec::new();
        for relative_path in ["src/event.rs", "src/lib.rs"] {
            self.collect_violations(relative_path, forbidden_fragments, &mut violations);
        }
        assert!(
            violations.is_empty(),
            "terminal prompt-gate vocabulary belongs to persona-terminal:\n{}",
            violations.join("\n")
        );
    }

    fn collect_violations(
        &self,
        relative_path: &str,
        forbidden_fragments: &[&str],
        violations: &mut Vec<String>,
    ) {
        let path = self.root.join(relative_path);
        let content = std::fs::read_to_string(&path).expect("scan source file");
        for fragment in forbidden_fragments {
            if content.contains(fragment) {
                violations.push(format!("{relative_path} contains {fragment}"));
            }
        }
    }
}

#[test]
fn niri_windows_observe_target_focus_state() {
    let windows = NiriWindows::from_json_slice(
        br#"[
          {
            "id": 10,
            "title": "Responder",
            "app_id": "org.wezfurlong.wezterm",
            "pid": 100,
            "workspace_id": 1,
            "is_focused": false,
            "is_floating": false,
            "is_urgent": false,
            "layout": {},
            "focus_timestamp": {"secs": 1, "nanos": 5}
          },
          {
            "id": 11,
            "title": "Initiator",
            "app_id": "org.wezfurlong.wezterm",
            "pid": 100,
            "workspace_id": 1,
            "is_focused": true,
            "is_floating": false,
            "is_urgent": false,
            "layout": {},
            "focus_timestamp": {"secs": 2, "nanos": 9}
          }
        ]"#,
    )
    .expect("windows decode");

    let observation = windows
        .observe(SystemTarget::niri_window(11), NiriWindowId::new(11))
        .expect("target observed");

    assert!(observation.focused);
    assert_eq!(observation.generation.into_u64(), 2_000_000_009);
}

#[test]
fn focus_tracker_filters_title_chatter_without_focus_change() {
    let target = SystemTarget::niri_window(10);
    let mut tracker = FocusTracker::new(target, NiriWindowId::new(10));
    tracker.accept(FocusObservation::new(target, false, 1_000_000_005));
    let event = NiriEvent::from_json_str(
        r#"{
          "WindowOpenedOrChanged": {
            "window": {
              "id": 10,
              "title": "Responder ⠙",
              "app_id": "org.wezfurlong.wezterm",
              "pid": 100,
              "workspace_id": 1,
              "is_focused": false,
              "is_floating": false,
              "is_urgent": false,
              "layout": {},
              "focus_timestamp": {"secs": 1, "nanos": 5}
            }
          }
        }"#,
    )
    .expect("event decodes");

    assert!(tracker.apply_event(&event).is_empty());
}

#[test]
fn focus_tracker_emits_when_target_focus_changes() {
    let target = SystemTarget::niri_window(10);
    let mut tracker = FocusTracker::new(target, NiriWindowId::new(10));
    tracker.accept(FocusObservation::new(target, false, 1_000_000_005));
    let event = NiriEvent::from_json_str(
        r#"{
          "WindowOpenedOrChanged": {
            "window": {
              "id": 10,
              "title": "Responder",
              "app_id": "org.wezfurlong.wezterm",
              "pid": 100,
              "workspace_id": 1,
              "is_focused": true,
              "is_floating": false,
              "is_urgent": false,
              "layout": {},
              "focus_timestamp": {"secs": 2, "nanos": 7}
            }
          }
        }"#,
    )
    .expect("event decodes");

    let observations = tracker.apply_event(&event);

    assert_eq!(observations.len(), 1);
    assert!(observations[0].focused);
    assert_eq!(observations[0].generation.into_u64(), 2_000_000_007);
}

#[test]
fn focus_tracker_uses_workspace_active_window_events() {
    let target = SystemTarget::niri_window(10);
    let windows = NiriWindows::from_json_slice(
        br#"[
          {
            "id": 10,
            "title": "Responder",
            "app_id": "org.wezfurlong.wezterm",
            "pid": 100,
            "workspace_id": 1,
            "is_focused": false,
            "is_floating": false,
            "is_urgent": false,
            "layout": {},
            "focus_timestamp": {"secs": 1, "nanos": 5}
          }
        ]"#,
    )
    .expect("windows decode");
    let mut tracker = FocusTracker::new(target, NiriWindowId::new(10));
    tracker.accept_window(
        windows
            .window(NiriWindowId::new(10))
            .expect("window exists"),
    );
    let event = NiriEvent::from_json_str(
        r#"{
          "WorkspaceActiveWindowChanged": {
            "workspace_id": 1,
            "active_window_id": 10
          }
        }"#,
    )
    .expect("event decodes");

    let observations = tracker.apply_event(&event);

    assert_eq!(observations.len(), 1);
    assert!(observations[0].focused);
}
