//! End-to-end UI tests: drive the real app through egui's test harness (simulated mouse
//! and keyboard, no window or GPU needed) and check what it shows and does.

use eframe::egui::{Event, Key, Modifiers, PointerButton, Pos2, Vec2};
use egui_kittest::Harness;
// `Queryable` provides `get_by_label` and friends on the harness.
use egui_kittest::kittest::Queryable;

use super::{DEFAULT_ROOT_TEXT, MindMapApp, Screen};
use crate::model::{MindMap, NodeId};

/// A harness around a fresh app that touches no files: settings, recovery and recent
/// files are all off, unlike `MindMapApp::new`.
fn harness() -> Harness<'static, MindMapApp> {
    Harness::builder()
        .with_size(Vec2::new(1200.0, 800.0))
        .build_eframe(|cc| {
            let mut app = MindMapApp::with_map(MindMap::new(DEFAULT_ROOT_TEXT));
            app.ctx = cc.egui_ctx.clone();
            app
        })
}

/// Open the template called `name` from the start screen.
fn open_template(harness: &mut Harness<'static, MindMapApp>, name: &str) {
    harness.get_by_label(name).click();
    harness.run();
    assert_eq!(harness.state().screen, Screen::Editor);
}

/// The node whose label is `text`.
fn node_named(app: &MindMapApp, text: &str) -> NodeId {
    app.map
        .iter()
        .find(|(_, node)| node.text() == text)
        .unwrap_or_else(|| panic!("no node called {text:?}"))
        .0
}

/// Where node `id` is drawn on screen.
fn center_of(app: &MindMapApp, id: NodeId) -> Pos2 {
    app.view.node_rect(id).expect("node is on screen").center()
}

#[test]
fn start_screen_lists_templates_and_ui_size_reads_left_to_right() {
    let harness = harness();

    // Regression check: a full-height separator once pushed the gallery out of view.
    let window = harness.get_by_label("Blank").rect().max.y;
    assert!(window < 800.0, "gallery starts below the window: {window}");
    for name in ["Project Plan", "SWOT Analysis", "Weekly Planner"] {
        harness.get_by_label(name);
    }
    harness.get_by_label("Open existing map...");
    // Regression check: in a right-to-left corner the buttons once came out reversed.
    let minus = harness.get_by_label("-").rect().center().x;
    let percent = harness.get_by_label("100%").rect().center().x;
    let plus = harness.get_by_label("+").rect().center().x;
    assert!(
        minus < percent && percent < plus,
        "{minus} {percent} {plus}"
    );
}

#[test]
fn clicking_a_template_opens_it() {
    let mut harness = harness();

    open_template(&mut harness, "Project Plan");

    let app = harness.state();
    assert_eq!(
        app.map.len(),
        app.start.template_map(2).expect("template").len()
    );
    assert!(!app.dirty);
    harness.get_by_label("Tidy up");
}

#[test]
fn tab_adds_a_child_that_typing_names_and_undo_removes() {
    let mut harness = harness();
    open_template(&mut harness, "Blank");

    harness.key_press(Key::Tab);
    harness.run();
    harness.event(Event::Text("First idea".to_owned()));
    harness.key_press(Key::Enter);
    harness.run();

    let child = node_named(harness.state(), "First idea");
    assert_eq!(
        harness.state().map.node(child).expect("child").parent(),
        Some(harness.state().map.root())
    );
    harness.get_by_label("Undo").click();
    harness.run();
    harness.get_by_label("Undo").click();
    harness.run();
    assert_eq!(harness.state().map.len(), 1);
}

#[test]
fn dragging_a_node_onto_another_reattaches_it() {
    let mut harness = harness();
    open_template(&mut harness, "Project Plan");
    let (risks, scope) = {
        let app = harness.state();
        (node_named(app, "Risks"), node_named(app, "Scope"))
    };
    let from = center_of(harness.state(), risks);
    let to = center_of(harness.state(), scope);

    harness.drag_at(from);
    harness.run();
    // Several moves, so egui recognises a drag rather than a click.
    for step in 1..=5_u8 {
        let t = f32::from(step) / 5.0;
        harness.hover_at(from + (to - from) * t);
        harness.run();
    }
    harness.drop_at(to);
    harness.run();

    let app = harness.state();
    assert_eq!(app.map.node(risks).expect("risks").parent(), Some(scope));
    assert!(app.history.can_undo());
}

#[test]
fn right_click_opens_the_node_menu_and_rename_works() {
    let mut harness = harness();
    open_template(&mut harness, "Project Plan");
    let goals = node_named(harness.state(), "Goals");
    let pos = center_of(harness.state(), goals);

    for pressed in [true, false] {
        harness.event(Event::PointerButton {
            pos,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    harness.run();
    harness.get_by_label("Notes and link...");
    harness.get_by_label("Colour #4e79a7");
    // The toolbar has a Rename button too; the menu's is the one lower down.
    harness
        .get_all_by_label("Rename")
        .max_by(|a, b| a.rect().center().y.total_cmp(&b.rect().center().y))
        .expect("menu has Rename")
        .click();
    harness.run();

    let app = harness.state();
    assert_eq!(app.selected, Some(goals));
    assert_eq!(app.editing.as_ref().map(|session| session.id), Some(goals));
}

#[test]
fn find_bar_jumps_between_matches() {
    let mut harness = harness();
    open_template(&mut harness, "Project Plan");

    harness.key_press_modifiers(Modifiers::COMMAND, Key::F);
    harness.run();
    harness.event(Event::Text("phase".to_owned()));
    harness.run();
    harness.get_by_label("1 of 3");
    harness.key_press(Key::F3);
    harness.run();

    harness.get_by_label("2 of 3");
    let app = harness.state();
    let selected = app
        .selected
        .and_then(|id| app.map.node(id))
        .expect("a match");
    assert_eq!(selected.text(), "Phase 2");
}

#[test]
fn ui_size_plus_button_enlarges_the_interface() {
    let mut harness = harness();

    harness.get_by_label("+").click();
    harness.run();

    harness.get_by_label("110%");
    assert!((harness.state().ui_zoom - 1.1).abs() < 1e-4);
}

#[test]
fn notes_panel_opens_for_the_selected_node() {
    let mut harness = harness();
    open_template(&mut harness, "Project Plan");
    let goals = node_named(harness.state(), "Goals");
    harness.hover_at(center_of(harness.state(), goals));
    harness.drag_at(center_of(harness.state(), goals));
    harness.drop_at(center_of(harness.state(), goals));
    harness.run();
    assert_eq!(harness.state().selected, Some(goals));

    harness.key_press_modifiers(Modifiers::COMMAND, Key::I);
    harness.run();

    harness.get_by_label("Open link");
    harness.get_by_label("Colour #edc948").click();
    harness.run();
    let color = harness.state().map.node(goals).expect("goals").color();
    assert_eq!(color.map(String::from).as_deref(), Some("#edc948"));
}
