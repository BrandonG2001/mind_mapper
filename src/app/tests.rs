//! Unit tests for the app's behaviour, run without a window.

use eframe::egui::{Rect, Vec2};

use super::canvas::take_plain_enter;
use super::*;
use crate::clipboard;
use crate::details::{self, DetailsEdit};
use crate::model::MapError;
use crate::start_screen::{StartChoice, TemplateId};
use crate::storage;
use crate::test_support::TempDir;

fn app_with_child() -> (MindMapApp, NodeId) {
    let mut app = MindMapApp::with_map(MindMap::new("root"));
    app.perform(Action::AddChild);
    let child = app.selected.expect("new child is selected");
    app.commit_edit();
    (app, child)
}

#[test]
fn add_child_selects_and_edits_new_node() {
    let mut app = MindMapApp::with_map(MindMap::new("root"));

    app.perform(Action::AddChild);

    let id = app.selected.expect("new node is selected");
    assert_eq!(app.editing.as_ref().map(|session| session.id), Some(id));
    assert_eq!(app.map.len(), 2);
    assert!(app.dirty);
    assert!(app.history.can_undo());
}

#[test]
fn blank_edit_keeps_default_label() {
    let (app, child) = app_with_child();

    assert_eq!(app.map.node(child).expect("child").text(), NEW_NODE_TEXT);
}

#[test]
fn commit_edit_trims_and_applies_text() {
    let (mut app, child) = app_with_child();
    app.start_edit(child, "  Idea  ".to_owned());

    app.commit_edit();

    assert_eq!(app.map.node(child).expect("child").text(), "Idea");
    assert!(app.editing.is_none());
}

#[test]
fn multi_line_labels_keep_inner_line_breaks() {
    let (mut app, child) = app_with_child();
    app.start_edit(child, " First line\r\nsecond line \n".to_owned());

    app.commit_edit();

    assert_eq!(
        app.map.node(child).expect("child").text(),
        "First line\nsecond line"
    );
}

#[test]
fn only_plain_enter_is_taken() {
    let key = |modifiers| Event::Key {
        key: Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    };
    let mut events = vec![key(Modifiers::SHIFT), Event::Text("x".to_owned())];
    assert!(!take_plain_enter(&mut events));
    assert_eq!(events.len(), 2);

    events.push(key(Modifiers::NONE));
    assert!(take_plain_enter(&mut events));

    assert_eq!(events.len(), 2);
}

#[test]
fn add_sibling_on_root_adds_child() {
    let mut app = MindMapApp::with_map(MindMap::new("root"));
    app.selected = Some(app.map.root());

    app.perform(Action::AddSibling);

    let id = app.selected.expect("new node is selected");
    assert_eq!(
        app.map.node(id).expect("node").parent(),
        Some(app.map.root())
    );
}

#[test]
fn delete_selects_parent_and_undo_restores() {
    let (mut app, child) = app_with_child();

    app.perform(Action::Delete);

    assert!(app.map.node(child).is_none());
    assert_eq!(app.selected, Some(app.map.root()));
    app.perform(Action::Undo);
    assert!(app.map.node(child).is_some());
    app.perform(Action::Redo);
    assert!(app.map.node(child).is_none());
}

#[test]
fn reparent_moves_branch_and_undo_restores_it() {
    let (mut app, child) = app_with_child();
    let root = app.map.root();
    let other = app
        .edit_map(|map| map.add_child(root, "other"))
        .expect("root exists");
    // Dragging records the undo point before anything moves.
    app.history.record(app.map.clone());

    app.reparent(&[child], other);

    assert_eq!(app.map.node(child).expect("child").parent(), Some(other));
    app.perform(Action::Undo);
    assert_eq!(app.map.node(child).expect("child").parent(), Some(root));
}

#[test]
fn invalid_reparent_reports_error() {
    let (mut app, child) = app_with_child();
    let root = app.map.root();

    app.reparent(&[root], child);

    assert_eq!(app.map.node(root).expect("root").parent(), None);
    assert_eq!(app.status, MapError::RootNode.to_string());
}

#[test]
fn tidy_is_one_undo_step() {
    let (mut app, child) = app_with_child();
    app.map
        .set_position(child, crate::model::Point::new(-999.0, 0.0))
        .expect("child exists");
    let before = app.map.clone();

    app.perform(Action::Tidy);
    assert_ne!(app.map, before);
    app.perform(Action::Undo);

    assert_eq!(app.map, before);
}

#[test]
fn collapsing_moves_hidden_selection_to_folded_node() {
    let (mut app, child) = app_with_child();
    let grandchild = app
        .edit_map(|map| map.add_child(child, "deep"))
        .expect("child exists");
    app.selected = Some(grandchild);

    app.toggle_collapse(child);

    assert!(app.map.node(child).expect("child").is_collapsed());
    assert_eq!(app.selected, Some(child));
    assert_eq!(app.fold_state(), (true, "Expand"));
    app.perform(Action::ToggleCollapse);
    assert!(!app.map.node(child).expect("child").is_collapsed());
}

#[test]
fn leaves_and_root_cannot_be_folded() {
    let (mut app, child) = app_with_child();
    app.selected = Some(child);
    assert_eq!(app.fold_state(), (false, "Collapse"));
    app.selected = Some(app.map.root());
    assert_eq!(app.fold_state(), (false, "Collapse"));
    let before = app.map.clone();

    app.toggle_collapse(child);
    app.toggle_collapse(app.map.root());

    assert_eq!(app.map, before);
}

#[test]
fn adding_child_to_folded_node_unfolds_it() {
    let (mut app, child) = app_with_child();
    app.edit_map(|map| map.add_child(child, "deep"))
        .expect("child exists");
    app.toggle_collapse(child);
    app.selected = Some(child);

    app.perform(Action::AddChild);

    assert!(!app.map.node(child).expect("child").is_collapsed());
    assert_eq!(app.map.node(child).expect("child").children().len(), 2);
}

#[test]
fn dropping_onto_folded_node_unfolds_it() {
    let (mut app, child) = app_with_child();
    let root = app.map.root();
    let target = app
        .edit_map(|map| map.add_child(root, "target"))
        .expect("root exists");
    app.edit_map(|map| map.add_child(target, "inside"))
        .expect("target exists");
    app.toggle_collapse(target);

    app.reparent(&[child], target);

    assert!(!app.map.node(target).expect("target").is_collapsed());
    assert_eq!(app.map.node(child).expect("child").parent(), Some(target));
}

#[test]
fn arrows_start_at_root_then_follow_the_map() {
    let (mut app, child) = app_with_child();
    app.selected = None;

    app.perform(Action::Navigate(Direction::Down));
    assert_eq!(app.selected, Some(app.map.root()));
    app.perform(Action::Navigate(Direction::Right));
    assert_eq!(app.selected, Some(child));
    assert_eq!(app.reveal, Some(child));
    // Nothing further out: the selection stays put.
    app.perform(Action::Navigate(Direction::Right));

    assert_eq!(app.selected, Some(child));
}

#[test]
fn scroll_into_view_pans_only_when_needed() {
    let (mut app, child) = app_with_child();
    let canvas = Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(800.0, 600.0));
    let position = app.map.node(child).expect("child").position();
    app.view.camera_mut().look_at(position, canvas.center());
    let centred = *app.view.camera();

    app.scroll_into_view(child, canvas);
    assert_eq!(*app.view.camera(), centred);
    app.view.camera_mut().pan(Vec2::new(5000.0, 0.0));
    app.scroll_into_view(child, canvas);

    let on_screen = app.view.camera().world_to_screen(position);
    assert!(canvas.contains(on_screen));
}

#[test]
fn export_reports_success_and_bad_extensions() {
    let dir = TempDir::new("app_export");
    let (mut app, _) = app_with_child();

    let finish = |app: &mut MindMapApp| {
        while !app.exports.is_empty() {
            app.poll_jobs();
            std::thread::yield_now();
        }
    };

    app.export_to(&dir.path().join("map.md"));
    assert_eq!(app.status, "Exporting map.md...");
    finish(&mut app);
    assert_eq!(app.status, "Exported map.md");
    assert!(dir.path().join("map.md").exists());
    app.export_to(&dir.path().join("map.txt"));
    finish(&mut app);

    assert!(app.status.starts_with("Could not export map.txt"));
}

#[test]
fn search_jumps_to_matches_and_unfolds_hidden_ones() {
    let (mut app, child) = app_with_child();
    app.edit_map(|map| map.set_text(child, "Needle"))
        .expect("child");
    let hidden = app
        .edit_map(|map| map.add_child(child, "another needle"))
        .expect("child");
    app.toggle_collapse(child);
    assert!(app.map.node(child).expect("child").is_collapsed());

    app.perform(Action::Find);
    app.search
        .as_mut()
        .expect("find bar open")
        .set_query("needle");
    app.search_command(SearchCommand::QueryChanged);
    assert_eq!(app.selected, Some(child));
    app.perform(Action::FindNext);

    assert_eq!(app.selected, Some(hidden));
    assert!(!app.map.node(child).expect("child").is_collapsed());
    assert_eq!(app.reveal, Some(hidden));
    app.perform(Action::Dismiss);
    assert!(app.search.is_none());
}

#[test]
fn copy_cut_and_paste_branches() {
    let ctx = egui::Context::default();
    let (mut app, child) = app_with_child();
    app.edit_map(|map| map.add_child(child, "grandchild"))
        .expect("child");
    let outline = clipboard::branches_outline(&app.map, &[child]);
    app.selected = Some(child);

    app.copy_branch(&ctx, true);
    assert!(app.map.node(child).is_none());
    app.selected = Some(app.map.root());
    app.paste_outline(&outline);

    let pasted = app.selected.expect("pasted branch selected");
    assert_eq!(clipboard::branches_outline(&app.map, &[pasted]), outline);
    assert_eq!(app.status, "Pasted 2 nodes");
    app.selected = Some(app.map.root());
    app.copy_branch(&ctx, true);
    assert!(app.status.starts_with("Select a branch to cut"));
    app.paste_outline("   ");
    assert!(app.status.starts_with("Nothing to paste"));
}

#[test]
fn details_edits_apply_as_one_undo_step_and_follow_undo() {
    let (mut app, child) = app_with_child();
    app.selected = Some(child);
    let mut panel = DetailsPanel::default();
    panel.follow(&app.map, Some(child));
    app.details = Some(panel);
    let red = details::PALETTE[3];

    app.apply_details(
        child,
        &[
            DetailsEdit::Note("first".to_owned()),
            DetailsEdit::Link(Some("https://example.com".to_owned())),
            DetailsEdit::Color(Some(red)),
        ],
    );
    let node = app.map.node(child).expect("child");
    assert_eq!(
        (node.note(), node.link(), node.color()),
        ("first", Some("https://example.com"), Some(red))
    );
    app.perform(Action::Undo);

    let node = app.map.node(child).expect("child");
    assert_eq!((node.note(), node.link(), node.color()), ("", None, None));
    // The panel reloaded, so it has nothing stale to re-apply.
    assert_eq!(
        app.details.as_ref().expect("panel").pending(&app.map),
        Vec::new()
    );
}

#[test]
fn toggling_details_opens_and_closes_the_panel() {
    let mut app = MindMapApp::with_map(MindMap::new("root"));

    app.perform(Action::ToggleDetails);
    assert!(app.details.is_some());
    app.perform(Action::ToggleDetails);

    assert!(app.details.is_none());
}

#[test]
fn root_cannot_be_deleted() {
    let mut app = MindMapApp::with_map(MindMap::new("root"));
    app.selected = Some(app.map.root());

    app.perform(Action::Delete);

    assert_eq!(app.map.len(), 1);
    assert!(!app.history.can_undo());
}

#[test]
fn undo_drops_selection_of_removed_node() {
    let (mut app, child) = app_with_child();
    app.selected = Some(child);

    app.perform(Action::Undo);
    app.perform(Action::Undo);

    assert_eq!(app.selected, None);
    assert_eq!(app.map.len(), 1);
}

#[test]
fn app_starts_on_start_screen() {
    let app = MindMapApp::with_map(MindMap::new("root"));

    assert_eq!(app.screen, Screen::Start { can_return: false });
    assert_eq!(app.title(), "Mind Mapper");
}

#[test]
fn choosing_a_template_opens_it_in_the_editor() {
    let mut app = MindMapApp::with_map(MindMap::new("root"));
    let expected = app
        .start
        .template_map(TemplateId::BuiltIn(2))
        .expect("template exists");

    app.choose(StartChoice::Template(TemplateId::BuiltIn(2)));

    assert_eq!(app.screen, Screen::Editor);
    assert_eq!(app.map, expected);
    assert!(!app.dirty);
    assert!(app.file_path.is_none());
    assert!(app.needs_fit);
}

#[test]
fn new_shows_start_screen_and_back_returns_unchanged() {
    let (mut app, child) = app_with_child();
    app.screen = Screen::Editor;

    app.perform(Action::New);
    assert_eq!(app.screen, Screen::Start { can_return: true });
    app.choose(StartChoice::Back);

    assert_eq!(app.screen, Screen::Editor);
    assert!(app.map.node(child).is_some());
}

#[test]
fn restore_opens_recovered_map_as_unsaved() {
    let dir = TempDir::new("app_restore");
    let mut app = MindMapApp::with_map(MindMap::new("root"));
    app.recovery = Some(RecoveryStore::for_this_window(dir.path()).expect("store"));
    let mut map = MindMap::new("recovered");
    map.add_child(map.root(), "idea").expect("root exists");
    app.recovered = Some(Recovered {
        map: map.clone(),
        source: Some(PathBuf::from("ideas.json")),
        files: None,
    });
    app.deferred_open = Some(PathBuf::from("other.json"));

    app.choose(StartChoice::Restore);

    assert_eq!(app.screen, Screen::Editor);
    assert_eq!(app.map, map);
    assert_eq!(app.file_path, Some(PathBuf::from("ideas.json")));
    assert!(app.dirty);
    // Copied into this window's own recovery file straight away.
    let copy = app
        .recovery
        .as_ref()
        .expect("store")
        .load()
        .expect("readable");
    assert_eq!(copy.expect("copy written").map, map);
    assert!(app.recovered.is_none());
    assert!(app.deferred_open.is_none());
}

#[test]
fn discard_recovery_deletes_orphan_and_opens_deferred_file() {
    let dir = TempDir::new("app_discard");
    // A window that crashed with unsaved work.
    let crashed = RecoveryStore::for_this_window(dir.path()).expect("store");
    crashed
        .save(&MindMap::new("recovered"), None)
        .expect("save succeeds");
    crashed.simulate_crash();
    let deferred = dir.path().join("deferred.json");
    storage::save_to_path(&MindMap::new("deferred"), &deferred).expect("save succeeds");
    let mut app = MindMapApp::with_map(MindMap::new("root"));
    app.recovery = Some(RecoveryStore::for_this_window(dir.path()).expect("store"));
    app.find_orphan();
    assert!(app.recovered.is_some());
    app.deferred_open = Some(deferred.clone());

    app.choose(StartChoice::DiscardRecovery);

    assert!(app.recovered.is_none());
    let recovery_files = std::fs::read_dir(dir.path())
        .expect("readable")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("recovery"))
        .count();
    assert_eq!(recovery_files, 0);
    assert_eq!(app.file_path, Some(deferred));
    assert_eq!(app.screen, Screen::Editor);
}

#[test]
fn autosave_waits_for_interval_then_writes_recovery_copy() {
    let dir = TempDir::new("app_autosave");
    let ctx = egui::Context::default();
    let (mut app, _) = app_with_child();
    app.recovery = Some(RecoveryStore::for_this_window(dir.path()).expect("store"));
    let saved_copy = |app: &MindMapApp| {
        app.recovery
            .as_ref()
            .expect("store set")
            .load()
            .expect("load succeeds")
    };
    // The app has just started (time 0), so the first interval has not elapsed.
    app.autosave(&ctx);
    assert!(saved_copy(&app).is_none());

    app.last_autosave = -AUTOSAVE_SECONDS;
    app.autosave(&ctx);
    assert!(app.recovery_job.is_some(), "written in the background");
    app.finish_recovery_job();

    assert_eq!(saved_copy(&app).expect("copy written").map, app.map);
    assert!(!app.autosave_pending);
}

#[test]
fn clearing_waits_for_a_background_copy_so_none_is_left() {
    let dir = TempDir::new("app_clear_waits");
    let ctx = egui::Context::default();
    let (mut app, _) = app_with_child();
    app.recovery = Some(RecoveryStore::for_this_window(dir.path()).expect("store"));
    app.last_autosave = -AUTOSAVE_SECONDS;
    app.autosave(&ctx);

    app.clear_recovery();

    assert!(app.recovery_job.is_none());
    let store = app.recovery.as_ref().expect("store");
    assert!(store.load().expect("readable").is_none());
}

#[test]
fn huge_maps_keep_fewer_undo_steps() {
    let mut map = MindMap::new("root");
    // About 8 MB of notes: the 256 MB budget then fits about 30 snapshots, not 200.
    let root = map.root();
    map.set_note(root, &"x".repeat(8 * 1024 * 1024))
        .expect("root");
    let mut app = MindMapApp::with_map(map);

    for step in 0..60 {
        app.edit_map(|map| map.set_text(root, &format!("step {step}")));
    }

    let kept = app.history.undo_steps();
    assert!((10..60).contains(&kept), "kept {kept} steps");
}

#[test]
fn only_changes_made_elsewhere_count_as_overwriting() {
    let dir = TempDir::new("app_overwrite");
    let path = dir.path().join("shared.json");
    storage::save_to_path(&MindMap::new("shared"), &path).expect("save succeeds");
    let mut app = MindMapApp::with_map(MindMap::new("root"));
    app.open_path(&path);
    assert!(!app.would_overwrite_changes(&path));

    // Our own save keeps the stamp current, so saving again is not a conflict.
    let root = app.map.root();
    app.edit_map(|map| map.set_text(root, "edited here"));
    app.save_to(path.clone());
    assert_eq!(app.status, "Saved shared.json");
    assert!(!app.would_overwrite_changes(&path));

    // Another window saves the same file.
    let mut other = MindMap::new("shared");
    other
        .add_child(other.root(), "from the other window")
        .expect("root");
    storage::save_to_path(&other, &path).expect("save succeeds");
    assert!(app.would_overwrite_changes(&path));

    // A different existing file is not our concern (the save dialog already asked), and
    // a file that was deleted is simply saved again.
    assert!(!app.would_overwrite_changes(&dir.path().join("elsewhere.json")));
    std::fs::remove_file(&path).expect("deletable");
    assert!(!app.would_overwrite_changes(&path));
}

#[test]
fn restored_work_asks_before_overwriting_its_file() {
    let dir = TempDir::new("app_restored_overwrite");
    let path = dir.path().join("ideas.json");
    storage::save_to_path(&MindMap::new("on disk"), &path).expect("save succeeds");
    let mut app = MindMapApp::with_map(MindMap::new("root"));
    app.recovered = Some(Recovered {
        map: MindMap::new("recovered"),
        source: Some(path.clone()),
        files: None,
    });

    app.choose(StartChoice::Restore);

    assert_eq!(app.file_path.as_deref(), Some(path.as_path()));
    assert!(app.would_overwrite_changes(&path));
}

#[test]
fn saving_over_changes_made_elsewhere_waits_for_an_answer() {
    let dir = TempDir::new("app_overwrite_question");
    let path = dir.path().join("shared.json");
    storage::save_to_path(&MindMap::new("shared"), &path).expect("save succeeds");
    let mut app = MindMapApp::with_map(MindMap::new("root"));
    app.open_path(&path);
    let elsewhere =
        storage::save_to_path(&MindMap::new("changed elsewhere"), &path).expect("save succeeds");

    app.save_to(path.clone());
    assert_eq!(app.question, Some(Question::Overwrite(path.clone())));
    app.answer(false);
    assert!(app.status.starts_with("Not saved"), "{}", app.status);
    assert_eq!(
        storage::current_stamp(&path).expect("readable"),
        Some(elsewhere)
    );

    app.save_to(path.clone());
    app.answer(true);
    assert!(app.question.is_none());
    assert_eq!(app.status, "Saved shared.json");
    let (saved, _) = storage::load_from_path(&path).expect("loads");
    assert_eq!(saved.node(saved.root()).expect("root").text(), "shared");
}

#[test]
fn unsaved_work_is_kept_until_discarding_is_agreed() {
    let dir = TempDir::new("app_discard_question");
    let path = dir.path().join("other.json");
    storage::save_to_path(&MindMap::new("other"), &path).expect("save succeeds");
    let (mut app, _) = app_with_child();
    assert!(app.dirty);

    app.discard_then(AfterDiscard::OpenPath(path.clone()));
    assert_eq!(
        app.question,
        Some(Question::Discard(AfterDiscard::OpenPath(path.clone())))
    );
    app.answer(false);
    assert!(app.question.is_none());
    assert_eq!(app.map.len(), 2);

    app.discard_then(AfterDiscard::OpenPath(path.clone()));
    app.answer(true);
    assert_eq!(app.file_path.as_deref(), Some(path.as_path()));
    assert_eq!(app.map.len(), 1);

    // Nothing unsaved now, so nothing to ask.
    app.discard_then(AfterDiscard::Template(TemplateId::BuiltIn(0)));
    assert!(app.question.is_none());
    assert_eq!(app.file_path, None);
}

#[test]
fn closing_goes_ahead_once_discarding_is_agreed() {
    let (mut app, _) = app_with_child();
    app.discard_then(AfterDiscard::Close);
    assert!(app.has_unsaved_work());
    app.answer(true);
    // So the close the window then requests is not stopped to ask again.
    assert!(!app.has_unsaved_work());
    assert!(!app.autosave_pending);
}

#[test]
fn saving_removes_recovery_copy() {
    let dir = TempDir::new("app_save_clears");
    let (mut app, _) = app_with_child();
    let store = RecoveryStore::for_this_window(dir.path()).expect("store");
    store.save(&app.map, None).expect("save succeeds");
    app.recovery = Some(store);

    app.save_to(dir.path().join("map.json"));

    assert!(!app.dirty);
    let store = app.recovery.as_ref().expect("store set");
    assert!(store.load().expect("load succeeds").is_none());
}

#[test]
fn saving_and_opening_update_recent_files() {
    let dir = TempDir::new("app_recent");
    let path = dir.path().join("ideas.json");
    let (mut app, _) = app_with_child();

    app.save_to(path.clone());
    assert_eq!(app.recent.paths(), std::slice::from_ref(&path));
    app.choose(StartChoice::OpenRecent(0));

    assert_eq!(app.file_path, Some(path));
    assert_eq!(app.screen, Screen::Editor);
}

#[test]
fn file_label_is_just_the_file_name() {
    let path = Path::new("some").join("folder").join("ideas.json");

    assert_eq!(file_label(&path), "ideas.json");
}

#[test]
fn settings_changes_are_saved_to_the_shared_file() {
    let dir = TempDir::new("app_settings");
    let ctx = egui::Context::default();
    let mut app = MindMapApp::with_map(MindMap::new("root"));
    app.settings_file = Some(SettingsFile::in_dir(dir.path()));
    // Another window adds a recent file meanwhile.
    SettingsFile::in_dir(dir.path())
        .update(|settings| settings.recent.add(Path::new("other.json")))
        .expect("update succeeds");

    app.sync_theme(&ctx);
    app.theme = ColorTheme::LightGrey;
    app.sync_theme(&ctx);
    app.save_to(dir.path().join("mine.json"));

    let saved = SettingsFile::in_dir(dir.path()).load();
    assert_eq!(saved.theme, ColorTheme::LightGrey);
    assert_eq!(saved.recent.paths().len(), 2, "both windows' files kept");
    assert_eq!(app.recent, saved.recent);
}

#[test]
fn missing_recent_file_is_removed() {
    let dir = TempDir::new("app_recent_missing");
    let mut app = MindMapApp::with_map(MindMap::new("root"));
    app.recent.add(&dir.path().join("gone.json"));

    app.choose(StartChoice::OpenRecent(0));

    assert_eq!(app.recent.paths(), &[] as &[PathBuf]);
    assert_eq!(app.screen, Screen::Start { can_return: false });
}

#[test]
fn out_of_range_template_is_ignored() {
    let mut app = MindMapApp::with_map(MindMap::new("root"));

    app.choose(StartChoice::Template(TemplateId::BuiltIn(usize::MAX)));

    assert_eq!(app.screen, Screen::Start { can_return: false });
}

#[test]
fn title_reflects_file_and_dirty_state() {
    let (mut app, _) = app_with_child();
    app.screen = Screen::Editor;
    assert_eq!(app.title(), "*Untitled - Mind Mapper");

    app.file_path = Some(PathBuf::from("ideas.json"));
    app.dirty = false;

    assert_eq!(app.title(), "ideas.json - Mind Mapper");
}

#[test]
fn sync_theme_installs_chosen_theme() {
    let mut app = MindMapApp::with_map(MindMap::new("root"));
    let ctx = egui::Context::default();

    app.theme = ColorTheme::LightPurple;
    app.sync_theme(&ctx);
    assert_eq!(ctx.theme(), egui::Theme::Light);
    app.theme = ColorTheme::DarkBlue;
    app.sync_theme(&ctx);

    assert_eq!(ctx.theme(), egui::Theme::Dark);
    assert_eq!(app.applied_theme, Some(ColorTheme::DarkBlue));
}

/// An app showing a root with three children, stacked top to bottom.
fn app_with_three_children() -> (MindMapApp, [NodeId; 3]) {
    let mut map = MindMap::new("root");
    let root = map.root();
    let ids = ["a", "b", "c"].map(|text| map.add_child(root, text).expect("root"));
    (MindMapApp::with_map(map), ids)
}

fn y_of(app: &MindMapApp, id: NodeId) -> f32 {
    app.map.node(id).expect("live").position().y
}

#[test]
fn alt_arrows_move_a_node_among_its_siblings() {
    let (mut app, [a, b, c]) = app_with_three_children();
    app.select(Some(a));

    app.perform(Action::Reorder(Direction::Down));
    assert!(y_of(&app, b) < y_of(&app, a) && y_of(&app, a) < y_of(&app, c));
    assert_eq!(app.selected, Some(a));
    assert_eq!(
        app.map.node(app.map.root()).expect("root").children(),
        &[b, a, c]
    );

    // Up twice: the second finds nothing above and changes nothing.
    app.perform(Action::Reorder(Direction::Up));
    app.perform(Action::Reorder(Direction::Up));
    assert!(y_of(&app, a) < y_of(&app, b));
    app.perform(Action::Undo);
    assert!(y_of(&app, b) < y_of(&app, a));
}

#[test]
fn ctrl_click_and_boxes_build_up_a_selection() {
    let (mut app, [a, b, c]) = app_with_three_children();
    app.select(Some(a));

    app.toggle_selected(b);
    assert_eq!(app.selected, Some(b));
    assert!(app.is_selected(a));
    assert_eq!(app.status, "2 nodes selected");

    // Focusing a selected node keeps the others; any other node starts over.
    app.focus(a);
    assert_eq!((app.selected, app.is_selected(b)), (Some(a), true));
    app.focus(c);
    assert_eq!(app.selection().collect::<Vec<_>>(), vec![c]);

    app.select_many([a, b], true);
    assert_eq!((app.selected, app.selection().count()), (Some(c), 3));
    app.toggle_selected(c);
    assert!(!app.is_selected(c));
    assert_eq!(app.selection().count(), 2);
    app.select_many([b], false);
    assert_eq!(app.selection().collect::<Vec<_>>(), vec![b]);
}

#[test]
fn selected_branches_skip_nodes_inside_other_selected_branches() {
    let (mut app, [a, _, c]) = app_with_three_children();
    let a1 = app.map.add_child(a, "a1").expect("a");
    app.select(Some(c));
    app.select_many([a1, a], true);

    assert_eq!(app.selected_branches(), vec![a, c]);
    assert_eq!(
        clipboard::branches_outline(&app.map, &app.selected_branches()),
        "- a\n  - a1\n- c\n"
    );
}

#[test]
fn deleting_a_selection_removes_every_branch_in_one_step() {
    let (mut app, [a, _, c]) = app_with_three_children();
    let root = app.map.root();
    let a1 = app.map.add_child(a, "a1").expect("a");
    app.select(Some(a1));
    app.select_many([a, c, root], true);

    app.perform(Action::Delete);

    // The central idea stays; the main node's nearest survivor is selected.
    assert_eq!(app.map.len(), 2);
    assert_eq!(app.selection().collect::<Vec<_>>(), vec![root]);
    app.perform(Action::Undo);
    assert_eq!(app.map.len(), 5);
}

#[test]
fn colouring_a_selection_colours_every_node_in_one_step() {
    let (mut app, [a, b, c]) = app_with_three_children();
    app.select(Some(a));
    app.toggle_selected(c);
    let color = details::PALETTE[2];

    app.color_selection(Some(color));

    let color_of = |app: &MindMapApp, id| app.map.node(id).expect("live").color();
    assert_eq!(color_of(&app, a), Some(color));
    assert_eq!(color_of(&app, c), Some(color));
    assert_eq!(color_of(&app, b), None);
    app.perform(Action::Undo);
    assert_eq!(color_of(&app, a), None);
    assert_eq!(color_of(&app, c), None);
}

#[test]
fn dropping_several_branches_reattaches_them_all() {
    let (mut app, [a, b, c]) = app_with_three_children();
    let b1 = app.map.add_child(b, "b1").expect("b");
    app.history.record(app.map.clone());

    app.reparent(&[a, b1, c], b);

    for id in [a, b1, c] {
        assert_eq!(app.map.node(id).expect("live").parent(), Some(b));
    }
    // b1 was already there and was left where it was.
    assert_eq!(app.map.node(b).expect("b").children(), &[b1, a, c]);
}

#[test]
fn folding_or_undoing_drops_nodes_that_are_gone_from_the_selection() {
    let (mut app, [a, b, _]) = app_with_three_children();
    let a1 = app.map.add_child(a, "a1").expect("a");
    app.select(Some(a1));
    app.toggle_selected(b);

    // a1 is hidden by folding a; the main node, b, stays.
    app.toggle_collapse(a);
    assert_eq!(app.selection().collect::<Vec<_>>(), vec![b]);

    app.perform(Action::AddChild);
    app.commit_edit();
    let added = app.selected.expect("new node selected");
    app.toggle_selected(b);
    assert_eq!(app.selected, Some(b));
    app.focus(added);
    app.perform(Action::Undo);
    assert_eq!(app.selection().collect::<Vec<_>>(), vec![b]);
}

#[test]
fn focus_mode_shows_one_branch_until_dismissed() {
    let (mut app, [a, b, _]) = app_with_three_children();
    let a1 = app.map.add_child(a, "a1").expect("a");
    app.select(Some(app.map.root()));
    app.perform(Action::ToggleFocus);
    assert_eq!(app.view.focus(), None, "{}", app.status);

    app.select(Some(a1));
    app.perform(Action::ToggleFocus);
    assert_eq!(app.view.focus(), Some(a1));
    app.perform(Action::Dismiss);
    assert_eq!(app.view.focus(), None);

    app.select(Some(a));
    app.toggle_selected(b);
    app.focus(a);
    app.perform(Action::ToggleFocus);
    assert_eq!(app.view.focus(), Some(a));
    // b is outside the branch, so it is no longer selected.
    assert_eq!(app.selection().collect::<Vec<_>>(), vec![a]);
    // The arrows stay inside: Left from the focused branch would go to its parent.
    app.perform(Action::Navigate(Direction::Left));
    assert_eq!(app.selected, Some(a));
    app.perform(Action::Navigate(Direction::Right));
    assert_eq!(app.selected, Some(a1));
}

#[test]
fn focus_mode_ends_when_something_outside_must_be_shown() {
    let (mut app, [a, b, _]) = app_with_three_children();
    app.select(Some(a));
    app.perform(Action::ToggleFocus);

    // A sibling of the focused branch is outside it.
    app.perform(Action::AddSibling);
    assert_eq!(app.view.focus(), None);
    app.commit_edit();

    app.select(Some(a));
    app.perform(Action::ToggleFocus);
    app.show_node(b);
    assert_eq!(app.view.focus(), None);

    app.select(Some(a));
    app.perform(Action::ToggleFocus);
    app.perform(Action::Delete);
    app.check_focus();
    assert_eq!(app.view.focus(), None);
}

#[test]
fn saved_templates_appear_on_the_start_screen_and_can_be_removed() {
    let dir = TempDir::new("app_templates");
    let (mut app, _) = app_with_three_children();
    app.template_folder = Some(crate::user_templates::TemplateFolder::in_dir(dir.path()));

    app.perform(Action::SaveTemplate);
    assert!(app.status.starts_with("Saved \"root\""), "{}", app.status);
    app.perform(Action::SaveTemplate);
    assert_eq!(app.status, "Replaced the template \"root\"");
    assert_eq!(app.start.saved().len(), 1);

    app.dirty = false;
    app.choose(StartChoice::Template(TemplateId::Saved(0)));
    assert_eq!(app.map.len(), 4);
    assert_eq!(app.file_path, None);

    app.choose(StartChoice::RemoveTemplate(0));
    assert!(matches!(
        app.question,
        Some(Question::RemoveTemplate { .. })
    ));
    app.answer(true);
    assert_eq!(app.start.saved(), []);
    assert_eq!(app.status, "Removed the template \"root\"");
}

#[test]
fn opml_and_freemind_files_open_as_new_unsaved_maps() {
    let dir = TempDir::new("app_import");
    let (mut app, _) = app_with_three_children();
    let outline = dir.path().join("plan.opml");
    std::fs::write(&outline, crate::formats::opml::write(&app.map)).expect("writable");
    let broken = dir.path().join("broken.mm");
    std::fs::write(&broken, "<map>").expect("writable");
    app.dirty = false;

    app.open_path(&outline);
    assert_eq!(
        app.status,
        "Imported plan.opml; Save keeps it as a mind map"
    );
    assert_eq!(app.map.len(), 4);
    // Saving must not overwrite the outline with this app's own format.
    assert_eq!(app.file_path, None);

    app.open_path(&broken);
    assert!(app.status.starts_with("Could not import"), "{}", app.status);
    assert_eq!(app.map.len(), 4);
}
