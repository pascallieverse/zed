use acp_thread::ThreadStatus;
use agent_ui::{AgentPanel, AgentPanelEvent};
use db::kvp::KEY_VALUE_STORE;
use editor::Editor;
use gpui::{
    App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, Hsla, MouseButton,
    MouseDownEvent, Pixels, Point, Render, SharedString, Subscription, Window, anchored, deferred,
    px,
};
use menu;
use project::Event as ProjectEvent;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use theme::ActiveTheme;
use ui::{ContextMenu, Tooltip, prelude::*};
use workspace::{
    MultiWorkspace, NewWorkspaceInWindow, Sidebar as WorkspaceSidebar, SidebarEvent, Workspace,
};

const SIDEBAR_WIDTH: Pixels = px(40.0);
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentThreadStatus {
    Running,
    Completed,
}

#[derive(Clone, Debug)]
struct AgentThreadInfo {
    title: SharedString,
    status: AgentThreadStatus,
}

struct WorkspaceEntry {
    index: usize,
    label: SharedString,
    initials: SharedString,
    full_path: SharedString,
    custom_color: Option<Hsla>,
    has_thread: bool,
    thread_running: bool,
}

impl WorkspaceEntry {
    fn new(
        index: usize,
        workspace: &Entity<Workspace>,
        customization: Option<&WorkspaceCustomization>,
        cx: &App,
    ) -> Self {
        let workspace_ref = workspace.read(cx);

        let worktrees: Vec<Arc<Path>> = workspace_ref
            .worktrees(cx)
            .filter(|worktree| worktree.read(cx).is_visible())
            .map(|worktree| worktree.read(cx).abs_path())
            .collect();

        let worktree_names: Vec<String> = worktrees
            .iter()
            .filter_map(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().to_string())
            })
            .collect();

        let derived_label: SharedString = if worktree_names.is_empty() {
            format!("Workspace {}", index + 1).into()
        } else {
            worktree_names.join(", ").into()
        };

        let label: SharedString = customization
            .and_then(|c| c.custom_name.as_ref())
            .map(|name| SharedString::from(name.clone()))
            .unwrap_or(derived_label);

        let full_path: SharedString = worktrees
            .iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join("\n")
            .into();

        let initials = compute_initials(&label);

        let custom_color = customization.and_then(|c| c.custom_color);

        let thread_info = Self::thread_info(workspace, cx);
        let has_thread = thread_info.is_some();
        let thread_running = thread_info
            .as_ref()
            .is_some_and(|info| info.status == AgentThreadStatus::Running);

        Self {
            index,
            label,
            initials,
            full_path,
            custom_color,
            has_thread,
            thread_running,
        }
    }

    fn thread_info(workspace: &Entity<Workspace>, cx: &App) -> Option<AgentThreadInfo> {
        let agent_panel = workspace.read(cx).panel::<AgentPanel>(cx)?;
        let agent_panel_ref = agent_panel.read(cx);

        let thread_view = agent_panel_ref.as_active_thread_view(cx)?.read(cx);
        let thread = thread_view.thread.read(cx);

        let title = thread.title();
        let status = match thread.status() {
            ThreadStatus::Generating => AgentThreadStatus::Running,
            ThreadStatus::Idle => AgentThreadStatus::Completed,
        };
        Some(AgentThreadInfo { title, status })
    }
}

fn compute_initials(name: &str) -> SharedString {
    let words: Vec<&str> = name.split_whitespace().collect();
    let result = match words.len() {
        0 => "??".to_string(),
        1 => {
            let first = words[0];
            let chars: Vec<char> = first.chars().collect();
            if chars.len() >= 2 {
                format!(
                    "{}{}",
                    chars[0].to_uppercase(),
                    chars[1].to_lowercase()
                )
            } else if chars.len() == 1 {
                chars[0].to_uppercase().to_string()
            } else {
                "??".to_string()
            }
        }
        _ => {
            let first_initial = words[0].chars().next().map(|c| c.to_uppercase().to_string());
            let second_initial = words[1].chars().next().map(|c| c.to_uppercase().to_string());
            match (first_initial, second_initial) {
                (Some(first), Some(second)) => format!("{first}{second}"),
                (Some(first), None) => first,
                _ => "??".to_string(),
            }
        }
    };
    SharedString::from(result)
}

fn deterministic_color(name: &str) -> gpui::Hsla {
    let hash = name.bytes().fold(0u32, |acc, byte| {
        acc.wrapping_mul(31).wrapping_add(byte as u32)
    });
    let hue = (hash % 360) as f32;
    gpui::hsla(hue / 360.0, 0.4, 0.3, 1.0)
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct WorkspaceCustomization {
    custom_name: Option<String>,
    custom_color: Option<Hsla>,
}

const WORKSPACE_CUSTOMIZATIONS_KEY: &str = "workspace_sidebar_customizations";

fn workspace_paths_key(workspace: &Workspace, cx: &App) -> String {
    let mut paths: Vec<String> = workspace
        .worktrees(cx)
        .filter(|worktree| worktree.read(cx).is_visible())
        .map(|worktree| worktree.read(cx).abs_path().to_string_lossy().to_string())
        .collect();
    paths.sort();
    paths.join("\n")
}

const PRESET_COLORS: &[(& str, Hsla)] = &[
    ("Red", Hsla { h: 0.0, s: 0.4, l: 0.3, a: 1.0 }),
    ("Orange", Hsla { h: 0.083, s: 0.4, l: 0.3, a: 1.0 }),
    ("Yellow", Hsla { h: 0.15, s: 0.4, l: 0.3, a: 1.0 }),
    ("Green", Hsla { h: 0.33, s: 0.4, l: 0.3, a: 1.0 }),
    ("Teal", Hsla { h: 0.5, s: 0.4, l: 0.3, a: 1.0 }),
    ("Blue", Hsla { h: 0.6, s: 0.4, l: 0.3, a: 1.0 }),
    ("Purple", Hsla { h: 0.75, s: 0.4, l: 0.3, a: 1.0 }),
    ("Pink", Hsla { h: 0.9, s: 0.4, l: 0.3, a: 1.0 }),
];

struct EditWorkspacePopup {
    paths_key: String,
    name_editor: Entity<Editor>,
    selected_color: Option<Hsla>,
    original_name: String,
    focus_handle: FocusHandle,
}

impl EventEmitter<DismissEvent> for EditWorkspacePopup {}

enum EditWorkspacePopupEvent {
    Saved {
        paths_key: String,
        name: Option<String>,
        color: Option<Hsla>,
    },
}

impl EventEmitter<EditWorkspacePopupEvent> for EditWorkspacePopup {}

impl EditWorkspacePopup {
    fn new(
        paths_key: String,
        current_name: &str,
        current_color: Option<Hsla>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name_editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_text(current_name, window, cx);
            editor
        });

        Self {
            paths_key,
            name_editor,
            selected_color: current_color,
            original_name: current_name.to_string(),
            focus_handle: cx.focus_handle(),
        }
    }

    fn cancel(&mut self, _: &menu::Cancel, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn confirm(&mut self, _: &menu::Confirm, _window: &mut Window, cx: &mut Context<Self>) {
        let name = self
            .name_editor
            .read(cx)
            .text(cx)
            .trim()
            .to_string();

        let custom_name = if name.is_empty() || name == self.original_name {
            None
        } else {
            Some(name)
        };

        cx.emit(EditWorkspacePopupEvent::Saved {
            paths_key: self.paths_key.clone(),
            name: custom_name,
            color: self.selected_color,
        });
        cx.emit(DismissEvent);
    }

    fn select_color(&mut self, color: Option<Hsla>, cx: &mut Context<Self>) {
        self.selected_color = color;
        cx.notify();
    }
}

impl Focusable for EditWorkspacePopup {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for EditWorkspacePopup {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected_color = self.selected_color;

        v_flex()
            .key_context("EditWorkspacePopup")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::confirm))
            .elevation_2(cx)
            .p_2()
            .gap_2()
            .w(px(220.))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().colors().text)
                    .child("Edit Workspace"),
            )
            .child(
                div()
                    .border_1()
                    .border_color(cx.theme().colors().border)
                    .rounded_md()
                    .child(self.name_editor.clone()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().colors().text_muted)
                    .child("Color"),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        div()
                            .id("color-auto")
                            .w(px(20.))
                            .h(px(20.))
                            .rounded_full()
                            .cursor_pointer()
                            .bg(cx.theme().colors().element_background)
                            .border_1()
                            .when(selected_color.is_none(), |this| {
                                this.border_color(cx.theme().colors().border_focused)
                                    .border_2()
                            })
                            .when(selected_color.is_some(), |this| {
                                this.border_color(cx.theme().colors().border)
                            })
                            .tooltip(Tooltip::text("Auto"))
                            .on_click(cx.listener(|this, _, _window, cx| {
                                this.select_color(None, cx);
                            })),
                    )
                    .children(PRESET_COLORS.iter().map(|(name, color)| {
                        let color = *color;
                        let name = SharedString::from(*name);
                        let is_selected = selected_color == Some(color);

                        div()
                            .id(SharedString::from(format!("color-{name}")))
                            .w(px(20.))
                            .h(px(20.))
                            .rounded_full()
                            .cursor_pointer()
                            .bg(color)
                            .border_1()
                            .when(is_selected, |this| {
                                this.border_color(cx.theme().colors().border_focused)
                                    .border_2()
                            })
                            .when(!is_selected, |this| {
                                this.border_color(gpui::hsla(0.0, 0.0, 1.0, 0.2))
                            })
                            .tooltip(Tooltip::text(name.clone()))
                            .on_click(cx.listener(move |this, _, _window, cx| {
                                this.select_color(Some(color), cx);
                            }))
                    })),
            )
            .child(
                div()
                    .id("save-workspace-edit")
                    .w_full()
                    .flex()
                    .justify_center()
                    .rounded_md()
                    .py_1()
                    .cursor_pointer()
                    .bg(cx.theme().colors().element_background)
                    .hover(|style| style.bg(cx.theme().colors().element_hover))
                    .text_sm()
                    .text_color(cx.theme().colors().text)
                    .child("Save")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            this.confirm(&menu::Confirm, window, cx);
                        }),
                    ),
            )
    }
}

#[derive(Clone)]
pub struct DraggedProjectIcon {
    pub index: usize,
}

impl Render for DraggedProjectIcon {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

pub struct Sidebar {
    multi_workspace: Entity<MultiWorkspace>,
    focus_handle: FocusHandle,
    entries: Vec<WorkspaceEntry>,
    active_workspace_index: usize,
    notified_workspaces: HashSet<usize>,
    customizations: HashMap<String, WorkspaceCustomization>,
    context_menu: Option<(Entity<ContextMenu>, Point<Pixels>, Subscription)>,
    edit_popup: Option<(Entity<EditWorkspacePopup>, Point<Pixels>, Vec<Subscription>)>,
    _subscription: Subscription,
    _project_subscriptions: Vec<Subscription>,
    _agent_panel_subscriptions: Vec<Subscription>,
    _thread_subscriptions: Vec<Subscription>,
    #[cfg(any(test, feature = "test-support"))]
    test_thread_infos: HashMap<usize, AgentThreadInfo>,
}

impl EventEmitter<SidebarEvent> for Sidebar {}

impl Sidebar {
    pub fn new(
        multi_workspace: Entity<MultiWorkspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscription = cx.observe_in(
            &multi_workspace,
            window,
            |this, multi_workspace, window, cx| {
                this.queue_refresh(multi_workspace, window, cx);
            },
        );

        let customizations = KEY_VALUE_STORE
            .read_kvp(WORKSPACE_CUSTOMIZATIONS_KEY)
            .ok()
            .flatten()
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();

        let mut this = Self {
            multi_workspace,
            focus_handle: cx.focus_handle(),
            entries: Vec::new(),
            active_workspace_index: 0,
            notified_workspaces: HashSet::new(),
            customizations,
            context_menu: None,
            edit_popup: None,
            _subscription: subscription,
            _project_subscriptions: Vec::new(),
            _agent_panel_subscriptions: Vec::new(),
            _thread_subscriptions: Vec::new(),
            #[cfg(any(test, feature = "test-support"))]
            test_thread_infos: HashMap::new(),
        };
        this.queue_refresh(this.multi_workspace.clone(), window, cx);
        this
    }

    fn subscribe_to_projects(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Subscription> {
        let projects: Vec<_> = self
            .multi_workspace
            .read(cx)
            .workspaces()
            .iter()
            .map(|workspace| workspace.read(cx).project().clone())
            .collect();

        projects
            .iter()
            .map(|project| {
                cx.subscribe_in(
                    project,
                    window,
                    |this, _project, event, window, cx| match event {
                        ProjectEvent::WorktreeAdded(_)
                        | ProjectEvent::WorktreeRemoved(_)
                        | ProjectEvent::WorktreeOrderChanged => {
                            this.queue_refresh(this.multi_workspace.clone(), window, cx);
                        }
                        _ => {}
                    },
                )
            })
            .collect()
    }

    fn subscribe_to_agent_panels(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Subscription> {
        let workspaces: Vec<_> = self.multi_workspace.read(cx).workspaces().to_vec();

        workspaces
            .iter()
            .map(|workspace| {
                if let Some(agent_panel) = workspace.read(cx).panel::<AgentPanel>(cx) {
                    cx.subscribe_in(
                        &agent_panel,
                        window,
                        |this, _, _event: &AgentPanelEvent, window, cx| {
                            this.queue_refresh(this.multi_workspace.clone(), window, cx);
                        },
                    )
                } else {
                    cx.observe_in(workspace, window, |this, _, window, cx| {
                        this.queue_refresh(this.multi_workspace.clone(), window, cx);
                    })
                }
            })
            .collect()
    }

    fn subscribe_to_threads(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Subscription> {
        let workspaces: Vec<_> = self.multi_workspace.read(cx).workspaces().to_vec();

        workspaces
            .iter()
            .filter_map(|workspace| {
                let agent_panel = workspace.read(cx).panel::<AgentPanel>(cx)?;
                let thread = agent_panel.read(cx).active_agent_thread(cx)?;
                Some(cx.observe_in(&thread, window, |this, _, window, cx| {
                    this.queue_refresh(this.multi_workspace.clone(), window, cx);
                }))
            })
            .collect()
    }

    fn build_entries(&self, multi_workspace: &MultiWorkspace, cx: &App) -> Vec<WorkspaceEntry> {
        #[allow(unused_mut)]
        let mut entries: Vec<WorkspaceEntry> = multi_workspace
            .workspaces()
            .iter()
            .enumerate()
            .map(|(index, workspace)| {
                let key = workspace_paths_key(workspace.read(cx), cx);
                let customization = self.customizations.get(&key);
                WorkspaceEntry::new(index, workspace, customization, cx)
            })
            .collect();

        #[cfg(any(test, feature = "test-support"))]
        for (index, info) in &self.test_thread_infos {
            if let Some(entry) = entries.get_mut(*index) {
                entry.has_thread = true;
                entry.thread_running = info.status == AgentThreadStatus::Running;
            }
        }

        entries
    }

    fn update_notifications(&mut self, entries: &[WorkspaceEntry]) {
        let old_statuses: HashMap<usize, bool> = self
            .entries
            .iter()
            .map(|entry| (entry.index, entry.thread_running))
            .collect();

        for entry in entries {
            if entry.has_thread
                && !entry.thread_running
                && entry.index != self.active_workspace_index
            {
                if old_statuses.get(&entry.index) == Some(&true) {
                    self.notified_workspaces.insert(entry.index);
                }
            }
        }
    }

    fn queue_refresh(
        &mut self,
        multi_workspace: Entity<MultiWorkspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.defer_in(window, move |this, _window, cx| {
            this._project_subscriptions = this.subscribe_to_projects(_window, cx);
            this._agent_panel_subscriptions = this.subscribe_to_agent_panels(_window, cx);
            this._thread_subscriptions = this.subscribe_to_threads(_window, cx);

            let (entries, active_index) = multi_workspace.read_with(cx, |multi_workspace, cx| {
                (
                    this.build_entries(multi_workspace, cx),
                    multi_workspace.active_workspace_index(),
                )
            });

            let had_notifications = !this.notified_workspaces.is_empty();

            this.update_notifications(&entries);

            if this.active_workspace_index != active_index {
                this.notified_workspaces.remove(&active_index);
            }
            this.active_workspace_index = active_index;
            this.entries = entries;

            let has_notifications = !this.notified_workspaces.is_empty();
            if had_notifications != has_notifications {
                multi_workspace.update(cx, |_, cx| cx.notify());
            }
            cx.notify();
        });
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_test_thread_info(
        &mut self,
        index: usize,
        title: SharedString,
        status: AgentThreadStatus,
    ) {
        self.test_thread_infos.insert(
            index,
            AgentThreadInfo { title, status },
        );
    }

    fn deploy_context_menu(
        &mut self,
        index: usize,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let multi_workspace = self.multi_workspace.clone();
        let workspace_count = self.entries.len();

        let sidebar_handle = cx.entity().downgrade();
        let edit_position = position;

        let context_menu = ContextMenu::build(window, cx, move |menu, _window, _cx| {
            let menu = {
                let sidebar_handle = sidebar_handle.clone();
                menu.entry("Edit Workspace", None, move |window, cx| {
                    sidebar_handle
                        .update(cx, |sidebar, cx| {
                            sidebar.open_edit_popup(index, edit_position, window, cx);
                        })
                        .ok();
                })
            };
            if workspace_count > 1 {
                let multi_workspace = multi_workspace.clone();
                menu.entry("Remove Workspace", None, move |window, cx| {
                    multi_workspace.update(cx, |mw, cx| {
                        mw.remove_workspace(index, window, cx);
                    });
                })
            } else {
                menu
            }
        });

        window.focus(&context_menu.focus_handle(cx), cx);
        let subscription = cx.subscribe(&context_menu, |this, _, _: &DismissEvent, cx| {
            this.context_menu.take();
            cx.notify();
        });
        self.context_menu = Some((context_menu, position, subscription));
        cx.notify();
    }

    fn open_edit_popup(
        &mut self,
        index: usize,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workspace = self
            .multi_workspace
            .read(cx)
            .workspaces()
            .get(index)
            .cloned();
        let Some(workspace) = workspace else { return };

        let paths_key = workspace_paths_key(workspace.read(cx), cx);

        let current_label = self
            .entries
            .get(index)
            .map(|entry| entry.label.to_string())
            .unwrap_or_default();

        let current_color = self
            .customizations
            .get(&paths_key)
            .and_then(|c| c.custom_color);

        let popup = cx.new(|cx| {
            EditWorkspacePopup::new(
                paths_key,
                &current_label,
                current_color,
                window,
                cx,
            )
        });

        let editor_focus = popup.read(cx).name_editor.focus_handle(cx);
        window.focus(&editor_focus, cx);

        let dismiss_subscription =
            cx.subscribe(&popup, |this, _, _: &DismissEvent, cx| {
                this.edit_popup.take();
                cx.notify();
            });

        let multi_workspace = self.multi_workspace.clone();
        let save_subscription =
            cx.subscribe(&popup, move |this, _, event: &EditWorkspacePopupEvent, cx| {
                match event {
                    EditWorkspacePopupEvent::Saved {
                        paths_key,
                        name,
                        color,
                    } => {
                        let customization = this
                            .customizations
                            .entry(paths_key.clone())
                            .or_default();
                        customization.custom_name = name.clone();
                        customization.custom_color = *color;

                        let multi_workspace_ref = multi_workspace.read(cx);
                        this.entries = this.build_entries(multi_workspace_ref, cx);

                        this.persist_customizations(cx);
                        cx.notify();
                    }
                }
            });

        self.edit_popup = Some((popup, position, vec![dismiss_subscription, save_subscription]));
        cx.notify();
    }

    fn persist_customizations(&self, cx: &mut Context<Self>) {
        if let Ok(json) = serde_json::to_string(&self.customizations) {
            cx.background_spawn(async move {
                KEY_VALUE_STORE
                    .write_kvp(WORKSPACE_CUSTOMIZATIONS_KEY.to_string(), json)
                    .await
                    .ok();
            })
            .detach();
        }
    }

    fn render_project_icon(
        &self,
        entry: &WorkspaceEntry,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let index = entry.index;
        let is_active = index == self.active_workspace_index;
        let has_notification = self.notified_workspaces.contains(&index);
        let has_context_menu = self
            .context_menu
            .as_ref()
            .is_some_and(|(_, _, _)| true);
        let initials = entry.initials.clone();
        let label = entry.label.clone();
        let full_path = entry.full_path.clone();
        let background_color = entry
            .custom_color
            .unwrap_or_else(|| deterministic_color(label.as_ref()));
        let multi_workspace = self.multi_workspace.clone();

        div()
            .id(("project-icon", index))
            .w(px(32.))
            .h(px(32.))
            .mx_auto()
            .mb_1()
            .rounded(px(8.))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .bg(background_color)
            .when(!has_context_menu, |this| {
                this.hover(|style| style.opacity(0.8))
            })
            .when(is_active, |this| {
                this.border_l(px(3.))
                    .border_color(cx.theme().colors().border_focused)
            })
            .when(has_notification, |this| {
                this.border_r(px(2.))
                    .border_color(gpui::hsla(0.08, 0.9, 0.5, 1.0))
            })
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(gpui::hsla(0.0, 0.0, 1.0, 0.95))
                    .child(initials),
            )
            .on_drag(
                DraggedProjectIcon { index },
                |dragged, _, _, cx| cx.new(|_| dragged.clone()),
            )
            .drag_over::<DraggedProjectIcon>(move |icon, dragged, _, cx| {
                if dragged.index != index {
                    let accent = cx.theme().colors().border_focused;
                    if dragged.index > index {
                        icon.border_t_2().border_color(accent)
                    } else {
                        icon.border_b_2().border_color(accent)
                    }
                } else {
                    icon
                }
            })
            .on_drop(cx.listener(
                move |this, dragged: &DraggedProjectIcon, window, cx| {
                    let from = dragged.index;
                    let to = index;
                    if from != to {
                        this.multi_workspace.update(cx, |mw, cx| {
                            mw.move_workspace(from, to, window, cx);
                        });
                    }
                },
            ))
            .on_click({
                let multi_workspace = multi_workspace.clone();
                cx.listener(move |_this, _event, window, cx| {
                    multi_workspace.update(cx, |mw, cx| {
                        mw.activate_index(index, window, cx);
                    });
                })
            })
            .on_mouse_down(MouseButton::Middle, {
                let multi_workspace = multi_workspace.clone();
                cx.listener(move |_this, _event, window, cx| {
                    multi_workspace.update(cx, |mw, cx| {
                        mw.remove_workspace(index, window, cx);
                    });
                })
            })
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.deploy_context_menu(index, event.position, window, cx);
                }),
            )
            .tooltip({
                let label_clone = label.clone();
                let full_path_clone = full_path.clone();
                move |_, cx| {
                    if full_path_clone.is_empty() {
                        Tooltip::with_meta(label_clone.clone(), None, "Not connected", cx)
                    } else {
                        Tooltip::with_meta(label_clone.clone(), None, full_path_clone.clone(), cx)
                    }
                }
            })
    }

    fn render_add_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let multi_workspace = self.multi_workspace.clone();

        div()
            .id("add-project")
            .w(px(32.))
            .h(px(32.))
            .mx_auto()
            .mt_1()
            .rounded(px(8.))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .bg(cx.theme().colors().element_background)
            .hover(|style| style.bg(cx.theme().colors().element_hover))
            .child(
                div()
                    .text_size(px(16.))
                    .text_color(cx.theme().colors().text_muted)
                    .child("+"),
            )
            .tooltip(|_window, cx| {
                Tooltip::for_action("New Workspace", &NewWorkspaceInWindow, cx)
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_this, _event, window, cx| {
                    multi_workspace.update(cx, |mw, cx| {
                        mw.create_workspace(window, cx);
                    });
                }),
            )
    }

}

impl WorkspaceSidebar for Sidebar {
    fn width(&self, _cx: &App) -> Pixels {
        SIDEBAR_WIDTH
    }

    fn set_width(&mut self, _width: Option<Pixels>, _cx: &mut Context<Self>) {}

    fn has_notifications(&self, _cx: &App) -> bool {
        !self.notified_workspaces.is_empty()
    }
}

impl Focusable for Sidebar {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Sidebar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let project_icons: Vec<_> = self
            .entries
            .iter()
            .map(|entry| self.render_project_icon(entry, cx).into_any_element())
            .collect();

        let top_padding = px(8.0);

        v_flex()
            .id("workspace-sidebar")
            .key_context("WorkspaceSidebar")
            .track_focus(&self.focus_handle)
            .h_full()
            .w(SIDEBAR_WIDTH)
            .pt(top_padding)
            .pb_2()
            .bg(cx.theme().colors().surface_background)
            .border_r_1()
            .border_color(cx.theme().colors().border)
            .children(project_icons.into_iter().map(|icon| {
                div()
                    .w_full()
                    .flex()
                    .justify_center()
                    .child(icon)
                    .into_any_element()
            }))
            .child(
                div()
                    .w_full()
                    .flex()
                    .justify_center()
                    .child(self.render_add_button(cx)),
            )
            .children(self.context_menu.as_ref().map(|(menu, position, _)| {
                deferred(
                    anchored()
                        .position(*position)
                        .anchor(gpui::Corner::TopLeft)
                        .child(menu.clone()),
                )
                .with_priority(3)
            }))
            .children(self.edit_popup.as_ref().map(|(popup, position, _)| {
                deferred(
                    div()
                        .absolute()
                        .size_full()
                        .top_0()
                        .left_0()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                if let Some((popup, _, _)) = this.edit_popup.take() {
                                    popup.update(cx, |_, cx| cx.emit(DismissEvent));
                                }
                            }),
                        )
                        .child(
                            anchored()
                                .snap_to_window_with_margin(px(8.))
                                .position(*position)
                                .anchor(gpui::Corner::TopLeft)
                                .child(
                                    div()
                                        .occlude()
                                        .on_mouse_down(MouseButton::Left, |_, _, _| {})
                                        .child(popup.clone()),
                                ),
                        ),
                )
                .with_priority(4)
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fs::{FakeFs, Fs};
    use gpui::TestAppContext;
    use settings::SettingsStore;

    fn init_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme::init(theme::LoadThemes::JustBase, cx);
            editor::init(cx);
        });
    }

    fn set_thread_info_and_refresh(
        sidebar: &Entity<Sidebar>,
        multi_workspace: &Entity<MultiWorkspace>,
        index: usize,
        title: &str,
        status: AgentThreadStatus,
        cx: &mut gpui::VisualTestContext,
    ) {
        sidebar.update_in(cx, |s, _window, _cx| {
            s.set_test_thread_info(index, SharedString::from(title.to_string()), status.clone());
        });
        multi_workspace.update_in(cx, |_, _window, cx| cx.notify());
        cx.run_until_parked();
    }

    fn has_notifications(sidebar: &Entity<Sidebar>, cx: &mut gpui::VisualTestContext) -> bool {
        sidebar.read_with(cx, |s, cx| s.has_notifications(cx))
    }

    #[gpui::test]
    async fn test_notification_on_running_to_completed_transition(cx: &mut TestAppContext) {
        init_test(cx);
        let fs = FakeFs::new(cx.executor());
        cx.update(|cx| <dyn Fs>::set_global(fs.clone(), cx));
        let project = project::Project::test(fs, [], cx).await;

        let (multi_workspace, cx) =
            cx.add_window_view(|window, cx| MultiWorkspace::test_new(project, window, cx));

        let sidebar = multi_workspace.update_in(cx, |_mw, window, cx| {
            let mw_handle = cx.entity();
            cx.new(|cx| Sidebar::new(mw_handle, window, cx))
        });
        multi_workspace.update_in(cx, |mw, window, cx| {
            mw.register_sidebar(sidebar.clone(), window, cx);
        });
        cx.run_until_parked();

        // Create a second workspace and switch to it so workspace 0 is background.
        multi_workspace.update_in(cx, |mw, window, cx| {
            mw.create_workspace(window, cx);
        });
        cx.run_until_parked();
        multi_workspace.update_in(cx, |mw, window, cx| {
            mw.activate_index(1, window, cx);
        });
        cx.run_until_parked();

        assert!(
            !has_notifications(&sidebar, cx),
            "should have no notifications initially"
        );

        set_thread_info_and_refresh(
            &sidebar,
            &multi_workspace,
            0,
            "Test Thread",
            AgentThreadStatus::Running,
            cx,
        );

        assert!(
            !has_notifications(&sidebar, cx),
            "Running status alone should not create a notification"
        );

        set_thread_info_and_refresh(
            &sidebar,
            &multi_workspace,
            0,
            "Test Thread",
            AgentThreadStatus::Completed,
            cx,
        );

        assert!(
            has_notifications(&sidebar, cx),
            "Running → Completed transition should create a notification"
        );
    }

    #[gpui::test]
    async fn test_no_notification_for_active_workspace(cx: &mut TestAppContext) {
        init_test(cx);
        let fs = FakeFs::new(cx.executor());
        cx.update(|cx| <dyn Fs>::set_global(fs.clone(), cx));
        let project = project::Project::test(fs, [], cx).await;

        let (multi_workspace, cx) =
            cx.add_window_view(|window, cx| MultiWorkspace::test_new(project, window, cx));

        let sidebar = multi_workspace.update_in(cx, |_mw, window, cx| {
            let mw_handle = cx.entity();
            cx.new(|cx| Sidebar::new(mw_handle, window, cx))
        });
        multi_workspace.update_in(cx, |mw, window, cx| {
            mw.register_sidebar(sidebar.clone(), window, cx);
        });
        cx.run_until_parked();

        // Workspace 0 is the active workspace — thread completes while
        // the user is already looking at it.
        set_thread_info_and_refresh(
            &sidebar,
            &multi_workspace,
            0,
            "Test Thread",
            AgentThreadStatus::Running,
            cx,
        );
        set_thread_info_and_refresh(
            &sidebar,
            &multi_workspace,
            0,
            "Test Thread",
            AgentThreadStatus::Completed,
            cx,
        );

        assert!(
            !has_notifications(&sidebar, cx),
            "should not notify for the workspace the user is already looking at"
        );
    }

    #[gpui::test]
    async fn test_notification_cleared_on_workspace_activation(cx: &mut TestAppContext) {
        init_test(cx);
        let fs = FakeFs::new(cx.executor());
        cx.update(|cx| <dyn Fs>::set_global(fs.clone(), cx));
        let project = project::Project::test(fs, [], cx).await;

        let (multi_workspace, cx) =
            cx.add_window_view(|window, cx| MultiWorkspace::test_new(project, window, cx));

        let sidebar = multi_workspace.update_in(cx, |_mw, window, cx| {
            let mw_handle = cx.entity();
            cx.new(|cx| Sidebar::new(mw_handle, window, cx))
        });
        multi_workspace.update_in(cx, |mw, window, cx| {
            mw.register_sidebar(sidebar.clone(), window, cx);
        });
        cx.run_until_parked();

        // Create a second workspace so we can switch away and back.
        multi_workspace.update_in(cx, |mw, window, cx| {
            mw.create_workspace(window, cx);
        });
        cx.run_until_parked();

        // Switch to workspace 1 so workspace 0 becomes a background workspace.
        multi_workspace.update_in(cx, |mw, window, cx| {
            mw.activate_index(1, window, cx);
        });
        cx.run_until_parked();

        // Thread on workspace 0 transitions Running → Completed while
        // the user is looking at workspace 1.
        set_thread_info_and_refresh(
            &sidebar,
            &multi_workspace,
            0,
            "Test Thread",
            AgentThreadStatus::Running,
            cx,
        );
        set_thread_info_and_refresh(
            &sidebar,
            &multi_workspace,
            0,
            "Test Thread",
            AgentThreadStatus::Completed,
            cx,
        );

        assert!(
            has_notifications(&sidebar, cx),
            "background workspace completion should create a notification"
        );

        // Switching back to workspace 0 should clear the notification.
        multi_workspace.update_in(cx, |mw, window, cx| {
            mw.activate_index(0, window, cx);
        });
        cx.run_until_parked();

        assert!(
            !has_notifications(&sidebar, cx),
            "notification should be cleared when workspace becomes active"
        );
    }
}
