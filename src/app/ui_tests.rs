//! End-to-end UI tests: drive the real app through egui's test harness (simulated mouse
//! and keyboard, no window or GPU needed) and check what it shows and does.

use eframe::egui::{Event, Key, Modifiers, PointerButton, Pos2, Vec2};
use egui_kittest::Harness;
// `Queryable` provides `get_by_label` and friends on the harness.
use egui_kittest::kittest::{NodeT, Queryable};

use super::outline::OutlineEvent;
use super::{DEFAULT_ROOT_TEXT, MindMapApp, Screen};
use crate::model::{MindMap, NodeId};
use crate::start_screen::TemplateId;

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
        app.start
            .template_map(TemplateId::BuiltIn(2))
            .expect("template")
            .len()
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
    // The menu's button also shows its shortcut, which tells it apart from the toolbar's.
    harness.get_by_label_contains("F2").click();
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

#[test]
fn starting_over_asks_in_the_window_before_discarding_edits() {
    let mut harness = harness();
    open_template(&mut harness, "Blank");
    harness.key_press(Key::Tab);
    harness.run();
    harness.key_press(Key::Enter);
    harness.run();
    assert!(harness.state().dirty);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::N);
    harness.run();

    // Regression check: the question was a system message box, which on Linux without
    // `zenity` never appeared and counted as "no", so unsaved windows could not close.
    harness.get_by_label("Blank").click();
    harness.run();
    harness.get_by_label("Discard the unsaved changes to this mind map?");
    harness.get_by_label("Cancel").click();
    harness.run();
    assert!(harness.state().question.is_none());
    assert_eq!(harness.state().map.len(), 2);

    harness.get_by_label("Blank").click();
    harness.run();
    harness.get_by_label("Discard changes").click();
    harness.run();
    assert_eq!(harness.state().screen, Screen::Editor);
    assert_eq!(harness.state().map.len(), 1);
}

/// Click node `id` while `modifiers` are held.
fn click_with(harness: &mut Harness<'static, MindMapApp>, id: NodeId, modifiers: Modifiers) {
    let pos = center_of(harness.state(), id);
    harness.event(Event::ModifiersChanged(modifiers));
    harness.hover_at(pos);
    for pressed in [true, false] {
        harness.event(Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers,
        });
    }
    harness.event(Event::ModifiersChanged(Modifiers::NONE));
    harness.run();
}

/// Drag with the primary button from `from` to `to` while `modifiers` are held.
fn drag_with(
    harness: &mut Harness<'static, MindMapApp>,
    from: Pos2,
    to: Pos2,
    modifiers: Modifiers,
) {
    harness.event(Event::ModifiersChanged(modifiers));
    harness.hover_at(from);
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
    harness.event(Event::ModifiersChanged(Modifiers::NONE));
    harness.run();
}

#[test]
fn ctrl_click_selects_several_nodes_and_delete_removes_them() {
    let mut harness = harness();
    open_template(&mut harness, "Project Plan");
    let (risks, scope) = {
        let app = harness.state();
        (node_named(app, "Risks"), node_named(app, "Scope"))
    };
    let nodes = harness.state().map.len();

    click_with(&mut harness, risks, Modifiers::NONE);
    click_with(&mut harness, scope, Modifiers::COMMAND);
    assert!(harness.state().is_selected(risks) && harness.state().is_selected(scope));
    harness.key_press(Key::Delete);
    harness.run();

    // Each branch had a node plus two children.
    assert_eq!(harness.state().map.len(), nodes - 6);
    harness.get_by_label("Undo").click();
    harness.run();
    assert_eq!(harness.state().map.len(), nodes);
}

#[test]
fn shift_drag_boxes_nodes_that_then_move_together() {
    let mut harness = harness();
    open_template(&mut harness, "Project Plan");
    let (timeline, team, mut phases) = {
        let app = harness.state();
        let phases = ["Phase 1", "Phase 2", "Phase 3"].map(|name| node_named(app, name));
        (node_named(app, "Timeline"), node_named(app, "Team"), phases)
    };
    phases.sort_by(|&a, &b| {
        let y = |id| center_of(harness.state(), id).y;
        y(a).total_cmp(&y(b))
    });
    let [first, second, third] = phases;
    let rect = |id| harness.state().view.node_rect(id).expect("on screen");
    // From empty canvas beyond the leaves (away from Timeline) to inside the second one.
    let outward = (rect(first).center().x - center_of(harness.state(), timeline).x).signum();
    let outer_edge = if outward > 0.0 {
        rect(first).max.x
    } else {
        rect(first).min.x
    };
    let origin = Pos2::new(outer_edge + outward * 20.0, rect(first).min.y - 4.0);
    let end = Pos2::new(rect(second).center().x, rect(second).max.y + 4.0);

    drag_with(&mut harness, origin, end, Modifiers::SHIFT);

    let app = harness.state();
    assert!(
        app.is_selected(first) && app.is_selected(second),
        "{}",
        app.status
    );
    assert!(!app.is_selected(third));
    assert_eq!(app.status, "2 nodes selected");

    // Dragging one of them carries both onto Team.
    let from = center_of(harness.state(), second);
    let to = center_of(harness.state(), team);
    drag_with(&mut harness, from, to, Modifiers::NONE);
    let app = harness.state();
    for id in [first, second] {
        assert_eq!(app.map.node(id).expect("live").parent(), Some(team));
    }
    assert_eq!(app.map.node(third).expect("live").parent(), Some(timeline));
}

#[test]
fn alt_down_moves_the_selected_node_below_its_sibling() {
    let mut harness = harness();
    open_template(&mut harness, "Project Plan");
    let (first, second) = {
        let app = harness.state();
        (
            node_named(app, "Objective 1"),
            node_named(app, "Objective 2"),
        )
    };
    click_with(&mut harness, first, Modifiers::NONE);

    harness.key_press_modifiers(Modifiers::ALT, Key::ArrowDown);
    harness.run();

    let app = harness.state();
    assert_eq!(app.selected, Some(first));
    let y = |id| app.map.node(id).expect("live").position().y;
    assert!(y(second) < y(first), "{} {}", y(second), y(first));
}

#[test]
fn f6_shows_only_the_selected_branch_until_escape() {
    let mut harness = harness();
    open_template(&mut harness, "Project Plan");
    let (goals, objective, risks) = {
        let app = harness.state();
        (
            node_named(app, "Goals"),
            node_named(app, "Objective 1"),
            node_named(app, "Risks"),
        )
    };
    click_with(&mut harness, goals, Modifiers::NONE);

    harness.key_press(Key::F6);
    harness.run();
    harness.run();

    let app = harness.state();
    assert!(app.view.node_rect(goals).is_some() && app.view.node_rect(objective).is_some());
    assert!(app.view.node_rect(risks).is_none());
    harness.get_by_label("Show all");
    harness.key_press(Key::Escape);
    harness.run();
    harness.run();
    assert!(harness.state().view.node_rect(risks).is_some());
    harness.get_by_label("Focus");
}

#[test]
fn a_saved_template_can_start_a_map_and_be_removed() {
    let dir = crate::test_support::TempDir::new("ui_templates");
    let mut harness = harness();
    harness.state_mut().template_folder =
        Some(crate::user_templates::TemplateFolder::in_dir(dir.path()));
    open_template(&mut harness, "Project Plan");
    let nodes = harness.state().map.len();

    harness.get_by_label("Save as template").click();
    harness.run();
    harness.key_press_modifiers(Modifiers::COMMAND, Key::N);
    harness.run();
    harness.get_by_label("Your templates");
    // The template is named after the map's central idea.
    harness.get_by_label("Project").click();
    harness.run();
    assert_eq!(harness.state().screen, Screen::Editor);
    assert_eq!(harness.state().map.len(), nodes);

    harness.key_press_modifiers(Modifiers::COMMAND, Key::N);
    harness.run();
    let card = harness.get_by_label("Project").rect().center();
    for pressed in [true, false] {
        harness.event(Event::PointerButton {
            pos: card,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    harness.run();
    harness.get_by_label("Remove template").click();
    harness.run();
    harness.get_by_label("Remove").click();
    harness.run();
    assert_eq!(harness.state().start.saved(), []);
    assert!(harness.query_by_label("Your templates").is_none());
}

#[test]
fn screen_readers_see_each_node_nested_like_the_map() {
    let mut harness = harness();
    open_template(&mut harness, "Project Plan");
    let risks = node_named(harness.state(), "Risks");
    {
        let objective = harness.get_by_label("Objective 1");
        let goals = objective.parent().expect("nested under its parent");
        assert_eq!(goals.accesskit_node().label().as_deref(), Some("Goals"));
        let project = goals.parent().expect("nested under the central idea");
        assert_eq!(project.accesskit_node().label().as_deref(), Some("Project"));
        let tree = project.parent().expect("inside the map's tree");
        assert_eq!(tree.accesskit_node().label().as_deref(), Some("Mind map"));
    }

    // Activating a node, as a screen reader does, selects it.
    harness.get_by_label("Risks").click_accesskit();
    harness.run();

    assert_eq!(harness.state().selected, Some(risks));
    let node = harness.get_by_label("Risks");
    assert_eq!(node.accesskit_node().is_selected(), Some(true));
    // Its outline on screen is where the node is drawn.
    let bounds = node.accesskit_node().bounding_box().expect("has bounds");
    let drawn = harness.state().view.node_rect(risks).expect("drawn");
    assert!(
        (bounds.x0 - f64::from(drawn.min.x)).abs() < 1.0,
        "{bounds:?} {drawn:?}"
    );
}

#[test]
fn link_to_from_the_menu_then_click_the_target() {
    let mut harness = harness();
    open_template(&mut harness, "Project Plan");
    let (risk, goals) = {
        let app = harness.state();
        (node_named(app, "Risk"), node_named(app, "Goals"))
    };
    let right_click = |harness: &mut Harness<'static, MindMapApp>, id| {
        let pos = center_of(harness.state(), id);
        for pressed in [true, false] {
            harness.event(Event::PointerButton {
                pos,
                button: PointerButton::Secondary,
                pressed,
                modifiers: Modifiers::NONE,
            });
        }
        harness.run();
    };

    right_click(&mut harness, risk);
    harness.get_by_label_contains("Link to...").click();
    harness.run();
    click_with(&mut harness, goals, Modifiers::NONE);

    assert_eq!(harness.state().links_of(risk), [(goals, true)]);
    assert_eq!(harness.state().status, "Linked \"Risk\" to \"Goals\"");
    // Screen readers hear about the link too.
    let described = harness.get_by_label("Risk").accesskit_node().description();
    assert_eq!(described.as_deref(), Some("Links to Goals"));

    // Label it from the menu: type, then Enter.
    right_click(&mut harness, goals);
    harness.get_by_label("Label link from \"Risk\"...").click();
    harness.run();
    harness.run();
    harness.event(Event::Text("threatens".to_owned()));
    harness.key_press(Key::Enter);
    harness.run();
    assert!(harness.state().label_edit.is_none());
    assert_eq!(harness.state().map.cross_links()[0].label(), "threatens");
    let described = harness.get_by_label("Risk").accesskit_node().description();
    assert_eq!(described.as_deref(), Some("Links to Goals (threatens)"));

    right_click(&mut harness, goals);
    harness.get_by_label("Remove link from \"Risk\"").click();
    harness.run();
    assert_eq!(harness.state().links_of(goals), []);
}

/// The outline's row (not the canvas node) labelled `label`.
fn outline_row<'h>(
    harness: &'h Harness<'static, MindMapApp>,
    label: &'h str,
) -> Option<egui_kittest::Node<'h>> {
    harness
        .query_all_by_label(label)
        .find(|node| node.accesskit_node().role() != eframe::egui::accesskit::Role::TreeItem)
}

#[test]
fn the_outline_lists_the_map_and_edits_it() {
    let mut harness = harness();
    open_template(&mut harness, "Project Plan");
    harness.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::O);
    harness.run();
    let (risks, mitigation, goals) = {
        let app = harness.state();
        (
            node_named(app, "Risks"),
            node_named(app, "Mitigation"),
            node_named(app, "Goals"),
        )
    };

    // Clicking a row selects its node.
    outline_row(&harness, "Risks").expect("row").click();
    harness.run();
    assert_eq!(harness.state().selected, Some(risks));

    // Renaming it right there in the list, as a double-click on the row does. (The test
    // harness does not produce double-clicks, so the rename is started directly.)
    harness
        .state_mut()
        .apply_outline_event(OutlineEvent::Rename(mitigation));
    harness.run();
    assert!(
        harness
            .state()
            .editing
            .as_ref()
            .is_some_and(|edit| edit.in_outline)
    );
    harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    harness.event(Event::Text("Contingency".to_owned()));
    harness.key_press(Key::Enter);
    harness.run();
    let text = harness
        .state()
        .map
        .node(mitigation)
        .expect("live")
        .text()
        .to_owned();
    assert_eq!(text, "Contingency");

    // The "-" button on the Goals row folds that branch, and its children leave the list.
    let row = outline_row(&harness, "Goals").expect("row").rect();
    harness
        .get_all_by_label("-")
        .find(|button| {
            let at = button.rect().center();
            (at.y - row.center().y).abs() < 2.0 && at.x < row.min.x
        })
        .expect("fold button on the Goals row")
        .click();
    harness.run();
    assert!(
        harness
            .state()
            .map
            .node(goals)
            .expect("live")
            .is_collapsed()
    );
    assert!(outline_row(&harness, "Objective 1").is_none());
}

#[test]
fn a_pasted_image_appears_on_the_selected_node() {
    let mut harness = harness();
    open_template(&mut harness, "Project Plan");
    let goals = node_named(harness.state(), "Goals");
    click_with(&mut harness, goals, Modifiers::NONE);
    let before = harness.state().view.node_rect(goals).expect("drawn");

    let pixels = vec![eframe::egui::Color32::BLUE; 120 * 80];
    let image = eframe::egui::ColorImage::new([120, 80], pixels);
    harness.event(Event::PasteImage(std::sync::Arc::new(image)));
    harness.run();

    assert!(
        harness
            .state()
            .map
            .node(goals)
            .expect("live")
            .picture()
            .is_some()
    );
    let after = harness.state().view.node_rect(goals).expect("drawn");
    assert!(
        after.height() > before.height() + 70.0,
        "{before:?} -> {after:?}"
    );
    assert!(after.width() >= 120.0);
}

/// Drag with the mouse from `from` to `to`, in steps egui recognises as a drag.
fn drag_between(harness: &mut Harness<'static, MindMapApp>, from: Pos2, to: Pos2) {
    harness.hover_at(from);
    harness.drag_at(from);
    harness.run();
    for step in 1..=5_u8 {
        let t = f32::from(step) / 5.0;
        harness.hover_at(from + (to - from) * t);
        harness.run();
    }
    harness.drop_at(to);
    harness.run();
}

#[test]
fn dragging_outline_rows_moves_branches() {
    let mut harness = harness();
    open_template(&mut harness, "Project Plan");
    harness.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::O);
    harness.run();
    let (risk, goals, first, second) = {
        let app = harness.state();
        (
            node_named(app, "Risk"),
            node_named(app, "Goals"),
            node_named(app, "Objective 1"),
            node_named(app, "Objective 2"),
        )
    };
    let row = |harness: &Harness<'static, MindMapApp>, label| {
        outline_row(harness, label).expect("row").rect()
    };

    // Onto the middle of a row: the branch becomes that node's child.
    let (from, to) = (
        row(&harness, "Risk").center(),
        row(&harness, "Goals").center(),
    );
    drag_between(&mut harness, from, to);
    assert_eq!(
        harness.state().map.node(risk).expect("live").parent(),
        Some(goals)
    );

    // Onto the top edge of a row: the branch goes just before that node.
    let target = row(&harness, "Objective 1");
    let from = row(&harness, "Objective 2").center();
    drag_between(
        &mut harness,
        from,
        Pos2::new(target.center().x, target.top() + 1.0),
    );
    let app = harness.state();
    assert_eq!(
        app.map.node(goals).expect("live").children(),
        &[second, first, risk]
    );
    // The canvas shows the same order: the moved node is now above its sibling.
    let y = |id| app.map.node(id).expect("live").position().y;
    assert!(y(second) < y(first), "{} {}", y(second), y(first));
}

#[test]
fn ctrl_h_replaces_text_across_the_map() {
    let mut harness = harness();
    open_template(&mut harness, "Project Plan");
    harness.get_by_label("Auto-save");

    harness.key_press_modifiers(Modifiers::COMMAND, Key::H);
    harness.run();
    // The keyboard is in the search field.
    harness.event(Event::Text("phase".to_owned()));
    harness.run();
    // Both fields are named after their labels, for screen readers.
    harness
        .query_all_by_role(eframe::egui::accesskit::Role::TextInput)
        .find(|field| field.accesskit_node().label().as_deref() == Some("Replace with"))
        .expect("the replacement field is named")
        .focus();
    harness.run();
    harness.event(Event::Text("Stage".to_owned()));
    harness.run();
    harness.get_by_label("Replace all").click();
    harness.run();

    let app = harness.state();
    assert_eq!(app.status, "Replaced 3 occurrences");
    for name in ["Stage 1", "Stage 2", "Stage 3"] {
        node_named(app, name);
    }
}
