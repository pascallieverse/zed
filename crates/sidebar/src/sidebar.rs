use acp_thread::ThreadStatus;
use agent_ui::{AgentPanel, AgentPanelEvent};
use gpui::{
    App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, MouseButton,
    MouseDownEvent, Pixels, Point, Render, SharedString, Subscription, Window, anchored, deferred,
    px,
};
use project::Event as ProjectEvent;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use theme::ActiveTheme;
use ui::{ContextMenu, Tooltip, prelude::*};
use workspace::{
    MultiWorkspace, NewWorkspaceInWindow, Sidebar as WorkspaceSidebar, SidebarEvent, Workspace,
};

const SIDEBAR_WIDTH: Pixels = px(40.0);
const TITLEBAR_TOP_PADDING: f32 = 44.0;

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
    has_thread: bool,
    thread_running: bool,
}

impl WorkspaceEntry {
    fn new(
        index: usize,
        workspace: &Entity<Workspace>,
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

        let label: SharedString = if worktree_names.is_empty() {
            format!("Workspace {}", index + 1).into()
        } else {
            worktree_names.join(", ").into()
        };

        let full_path: SharedString = worktrees
            .iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join("\n")
            .into();

        let initials = compute_initials(&label);

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
    context_menu: Option<(Entity<ContextMenu>, Point<Pixels>, Subscription)>,
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

        let mut this = Self {
            multi_workspace,
            focus_handle: cx.focus_handle(),
            entries: Vec::new(),
            active_workspace_index: 0,
            notified_workspaces: HashSet::new(),
            context_menu: None,
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
            .map(|(index, workspace)| WorkspaceEntry::new(index, workspace, cx))
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

        let context_menu = ContextMenu::build(window, cx, move |menu, _window, _cx| {
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

    fn render_project_icon(
        &self,
        entry: &WorkspaceEntry,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let index = entry.index;
        let is_active = index == self.active_workspace_index;
        let has_notification = self.notified_workspaces.contains(&index);
        let initials = entry.initials.clone();
        let label = entry.label.clone();
        let full_path = entry.full_path.clone();
        let background_color = deterministic_color(label.as_ref());
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
            .hover(|style| style.opacity(0.8))
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
            .when(!full_path.is_empty(), |this| {
                let label_clone = label.clone();
                let full_path_clone = full_path.clone();
                this.tooltip(move |_, cx| {
                    Tooltip::with_meta(label_clone.clone(), None, full_path_clone.clone(), cx)
                })
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

        let top_padding = if cfg!(target_os = "macos") && !window.is_fullscreen() {
            px(TITLEBAR_TOP_PADDING)
        } else {
            px(8.0)
        };

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
