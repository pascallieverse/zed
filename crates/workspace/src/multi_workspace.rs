use anyhow::Result;
use gpui::{
    Animation, AnimationExt, AnyView, App, Context, DragMoveEvent, Entity, EntityId,
    EventEmitter, FocusHandle, Focusable, ManagedView, MouseButton, Pixels, Render, Subscription,
    Task, Tiling, Window, WindowId, actions, deferred, ease_out_quint, px,
};
use project::Project;
use std::path::PathBuf;
use std::time::Duration;
use ui::prelude::*;

const SIDEBAR_RESIZE_HANDLE_SIZE: Pixels = px(6.0);

use crate::{
    DockPosition, Item, ModalView, Panel, Workspace, WorkspaceId, client_side_decorations,
};

actions!(
    multi_workspace,
    [
        /// Creates a new workspace within the current window.
        NewWorkspaceInWindow,
        /// Switches to the next workspace within the current window.
        NextWorkspaceInWindow,
        /// Switches to the previous workspace within the current window.
        PreviousWorkspaceInWindow,
        /// Closes the active workspace within the current window.
        RemoveActiveWorkspace,
        /// Toggles the workspace switcher sidebar.
        ToggleWorkspaceSidebar,
        /// Moves focus to or from the workspace sidebar without closing it.
        FocusWorkspaceSidebar,
    ]
);

pub enum SidebarEvent {
    Open,
    Close,
}

pub trait Sidebar: EventEmitter<SidebarEvent> + Focusable + Render + Sized {
    fn width(&self, cx: &App) -> Pixels;
    fn set_width(&mut self, width: Option<Pixels>, cx: &mut Context<Self>);
    fn has_notifications(&self, cx: &App) -> bool;
}

pub trait SidebarHandle: 'static + Send + Sync {
    fn width(&self, cx: &App) -> Pixels;
    fn set_width(&self, width: Option<Pixels>, cx: &mut App);
    fn focus_handle(&self, cx: &App) -> FocusHandle;
    fn focus(&self, window: &mut Window, cx: &mut App);
    fn has_notifications(&self, cx: &App) -> bool;
    fn to_any(&self) -> AnyView;
    fn entity_id(&self) -> EntityId;
}

#[derive(Clone)]
pub struct DraggedSidebar;

impl Render for DraggedSidebar {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

impl<T: Sidebar> SidebarHandle for Entity<T> {
    fn width(&self, cx: &App) -> Pixels {
        self.read(cx).width(cx)
    }

    fn set_width(&self, width: Option<Pixels>, cx: &mut App) {
        self.update(cx, |this, cx| this.set_width(width, cx))
    }

    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.read(cx).focus_handle(cx)
    }

    fn focus(&self, window: &mut Window, cx: &mut App) {
        let handle = self.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    }

    fn has_notifications(&self, cx: &App) -> bool {
        self.read(cx).has_notifications(cx)
    }

    fn to_any(&self) -> AnyView {
        self.clone().into()
    }

    fn entity_id(&self) -> EntityId {
        Entity::entity_id(self)
    }
}

const CROSSFADE_DURATION: Duration = Duration::from_millis(150);

pub struct MultiWorkspace {
    window_id: WindowId,
    workspaces: Vec<Entity<Workspace>>,
    active_workspace_index: usize,
    previous_workspace_index: Option<usize>,
    transition_id: usize,
    _transition_cleanup: Option<Task<()>>,
    sidebar: Option<Box<dyn SidebarHandle>>,
    sidebar_open: bool,
    _sidebar_subscription: Option<Subscription>,
}

impl MultiWorkspace {
    pub fn new(workspace: Entity<Workspace>, window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self {
            window_id: window.window_handle().window_id(),
            workspaces: vec![workspace],
            active_workspace_index: 0,
            previous_workspace_index: None,
            transition_id: 0,
            _transition_cleanup: None,
            sidebar: None,
            sidebar_open: false,
            _sidebar_subscription: None,
        }
    }

    pub fn register_sidebar<T: Sidebar>(
        &mut self,
        sidebar: Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let subscription =
            cx.subscribe_in(&sidebar, window, |this, _, event, window, cx| match event {
                SidebarEvent::Open => this.toggle_sidebar(window, cx),
                SidebarEvent::Close => {
                    this.close_sidebar(window, cx);
                }
            });
        self.sidebar = Some(Box::new(sidebar));
        self._sidebar_subscription = Some(subscription);
    }

    pub fn sidebar(&self) -> Option<&dyn SidebarHandle> {
        self.sidebar.as_deref()
    }

    pub fn sidebar_open(&self) -> bool {
        self.sidebar_open && self.sidebar.is_some()
    }

    pub fn sidebar_width(&self, cx: &App) -> Pixels {
        self.sidebar
            .as_ref()
            .map_or(px(0.), |s| s.width(cx))
    }

    pub fn sidebar_has_notifications(&self, cx: &App) -> bool {
        self.sidebar
            .as_ref()
            .map_or(false, |s| s.has_notifications(cx))
    }

    pub fn toggle_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sidebar_open {
            self.close_sidebar(window, cx);
        } else {
            self.open_sidebar(cx);
            if let Some(sidebar) = &self.sidebar {
                sidebar.focus(window, cx);
            }
        }
    }

    pub fn focus_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sidebar_open {
            let sidebar_is_focused = self
                .sidebar
                .as_ref()
                .is_some_and(|s| s.focus_handle(cx).contains_focused(window, cx));

            if sidebar_is_focused {
                let pane = self.workspace().read(cx).active_pane().clone();
                let pane_focus = pane.read(cx).focus_handle(cx);
                window.focus(&pane_focus, cx);
            } else if let Some(sidebar) = &self.sidebar {
                sidebar.focus(window, cx);
            }
        } else {
            self.open_sidebar(cx);
            if let Some(sidebar) = &self.sidebar {
                sidebar.focus(window, cx);
            }
        }
    }

    pub fn open_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_open = true;
        for workspace in &self.workspaces {
            workspace.update(cx, |workspace, cx| {
                workspace.set_workspace_sidebar_open(true, cx);
            });
        }
        self.serialize(cx);
        cx.notify();
    }

    fn close_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_open = false;
        for workspace in &self.workspaces {
            workspace.update(cx, |workspace, cx| {
                workspace.set_workspace_sidebar_open(false, cx);
            });
        }
        let pane = self.workspace().read(cx).active_pane().clone();
        let pane_focus = pane.read(cx).focus_handle(cx);
        window.focus(&pane_focus, cx);
        self.serialize(cx);
        cx.notify();
    }

    pub fn is_sidebar_open(&self) -> bool {
        self.sidebar_open
    }

    pub fn workspace(&self) -> &Entity<Workspace> {
        &self.workspaces[self.active_workspace_index]
    }

    pub fn workspaces(&self) -> &[Entity<Workspace>] {
        &self.workspaces
    }

    pub fn active_workspace_index(&self) -> usize {
        self.active_workspace_index
    }

    pub fn activate(&mut self, workspace: Entity<Workspace>, cx: &mut Context<Self>) {
        let index = self.add_workspace(workspace, cx);
        if self.active_workspace_index != index {
            self.active_workspace_index = index;
            self.serialize(cx);
            cx.notify();
        }
    }

    /// Adds a workspace to this window without changing which workspace is active.
    /// Returns the index of the workspace (existing or newly inserted).
    pub fn add_workspace(&mut self, workspace: Entity<Workspace>, cx: &mut Context<Self>) -> usize {
        if let Some(index) = self.workspaces.iter().position(|w| *w == workspace) {
            index
        } else {
            if self.sidebar_open {
                workspace.update(cx, |workspace, cx| {
                    workspace.set_workspace_sidebar_open(true, cx);
                });
            }
            self.workspaces.push(workspace);
            if self.workspaces.len() >= 2 {
                for workspace in &self.workspaces {
                    workspace.update(cx, |workspace, _cx| {
                        workspace.set_preserve_session(true);
                    });
                }
            }
            cx.notify();
            self.workspaces.len() - 1
        }
    }

    pub fn activate_index(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        debug_assert!(
            index < self.workspaces.len(),
            "workspace index out of bounds"
        );
        if index == self.active_workspace_index {
            return;
        }
        self.previous_workspace_index = Some(self.active_workspace_index);
        self.active_workspace_index = index;
        self.transition_id += 1;
        self._transition_cleanup = Some(cx.spawn_in(window, {
            let transition_id = self.transition_id;
            async move |this, cx| {
                cx.background_executor().timer(CROSSFADE_DURATION).await;
                this.update_in(cx, |this, _window, cx| {
                    if this.transition_id == transition_id {
                        this.previous_workspace_index = None;
                        cx.notify();
                    }
                })
                .ok();
            }
        }));
        self.serialize(cx);
        self.focus_active_workspace(window, cx);
        cx.notify();
    }

    pub fn activate_next_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.workspaces.len() > 1 {
            let next_index = (self.active_workspace_index + 1) % self.workspaces.len();
            self.activate_index(next_index, window, cx);
        }
    }

    pub fn activate_previous_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.workspaces.len() > 1 {
            let prev_index = if self.active_workspace_index == 0 {
                self.workspaces.len() - 1
            } else {
                self.active_workspace_index - 1
            };
            self.activate_index(prev_index, window, cx);
        }
    }

    fn serialize(&self, cx: &mut App) {
        let window_id = self.window_id;
        let state = crate::persistence::model::MultiWorkspaceState {
            active_workspace_id: self.workspace().read(cx).database_id(),
            sidebar_open: self.sidebar_open,
        };
        cx.background_spawn(async move {
            crate::persistence::write_multi_workspace_state(window_id, state).await;
        })
        .detach();
    }

    fn focus_active_workspace(&self, window: &mut Window, cx: &mut App) {
        // If a dock panel is zoomed, focus it instead of the center pane.
        // Otherwise, focusing the center pane triggers dismiss_zoomed_items_to_reveal
        // which closes the zoomed dock.
        let focus_handle = {
            let workspace = self.workspace().read(cx);
            let mut target = None;
            for dock in workspace.all_docks() {
                let dock = dock.read(cx);
                if dock.is_open() {
                    if let Some(panel) = dock.active_panel() {
                        if panel.is_zoomed(window, cx) {
                            target = Some(panel.panel_focus_handle(cx));
                            break;
                        }
                    }
                }
            }
            target.unwrap_or_else(|| {
                let pane = workspace.active_pane().clone();
                pane.read(cx).focus_handle(cx)
            })
        };
        window.focus(&focus_handle, cx);
    }

    pub fn panel<T: Panel>(&self, cx: &App) -> Option<Entity<T>> {
        self.workspace().read(cx).panel::<T>(cx)
    }

    pub fn active_modal<V: ManagedView + 'static>(&self, cx: &App) -> Option<Entity<V>> {
        self.workspace().read(cx).active_modal::<V>(cx)
    }

    pub fn add_panel<T: Panel>(
        &mut self,
        panel: Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace().update(cx, |workspace, cx| {
            workspace.add_panel(panel, window, cx);
        });
    }

    pub fn focus_panel<T: Panel>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<T>> {
        self.workspace()
            .update(cx, |workspace, cx| workspace.focus_panel::<T>(window, cx))
    }

    pub fn toggle_modal<V: ModalView, B>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        build: B,
    ) where
        B: FnOnce(&mut Window, &mut gpui::Context<V>) -> V,
    {
        self.workspace().update(cx, |workspace, cx| {
            workspace.toggle_modal(window, cx, build);
        });
    }

    pub fn toggle_dock(
        &mut self,
        dock_side: DockPosition,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace().update(cx, |workspace, cx| {
            workspace.toggle_dock(dock_side, window, cx);
        });
    }

    pub fn active_item_as<I: 'static>(&self, cx: &App) -> Option<Entity<I>> {
        self.workspace().read(cx).active_item_as::<I>(cx)
    }

    pub fn items_of_type<'a, T: Item>(
        &'a self,
        cx: &'a App,
    ) -> impl 'a + Iterator<Item = Entity<T>> {
        self.workspace().read(cx).items_of_type::<T>(cx)
    }

    pub fn database_id(&self, cx: &App) -> Option<WorkspaceId> {
        self.workspace().read(cx).database_id()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_random_database_id(&mut self, cx: &mut Context<Self>) {
        self.workspace().update(cx, |workspace, _cx| {
            workspace.set_random_database_id();
        });
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_new(project: Entity<Project>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let workspace = cx.new(|cx| Workspace::test_new(project, window, cx));
        Self::new(workspace, window, cx)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_add_workspace(
        &mut self,
        project: Entity<Project>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<Workspace> {
        let workspace = cx.new(|cx| Workspace::test_new(project, window, cx));
        self.activate(workspace.clone(), cx);
        workspace
    }

    pub fn create_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let app_state = self.workspace().read(cx).app_state().clone();
        let project = Project::local(
            app_state.client.clone(),
            app_state.node_runtime.clone(),
            app_state.user_store.clone(),
            app_state.languages.clone(),
            app_state.fs.clone(),
            None,
            project::LocalProjectFlags::default(),
            cx,
        );
        let new_workspace = cx.new(|cx| Workspace::new(None, project, app_state, window, cx));
        self.activate(new_workspace.clone(), cx);
        self.focus_active_workspace(window, cx);

        let workspace_weak = new_workspace.downgrade();
        cx.spawn_in(window, async move |_this, cx| {
            if let Ok(id) = crate::persistence::DB.next_id().await {
                workspace_weak
                    .update_in(cx, |workspace, window, cx| {
                        workspace.set_database_id(id);
                        workspace.serialize_workspace(window, cx);
                    })
                    .ok();
            }
        })
        .detach();
    }

    pub fn remove_active_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.remove_workspace(self.active_workspace_index, window, cx);
    }

    pub fn remove_workspace(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.workspaces.len() <= 1 || index >= self.workspaces.len() {
            return;
        }

        self.workspaces.remove(index);

        if self.workspaces.len() == 1 {
            self.workspaces[0].update(cx, |workspace, _cx| {
                workspace.set_preserve_session(false);
            });
        }

        if self.active_workspace_index >= self.workspaces.len() {
            self.active_workspace_index = self.workspaces.len() - 1;
        } else if self.active_workspace_index > index {
            self.active_workspace_index -= 1;
        }

        self.focus_active_workspace(window, cx);
        self.serialize(cx);
        cx.notify();
    }

    pub fn move_workspace(
        &mut self,
        from: usize,
        to: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if from >= self.workspaces.len() || to >= self.workspaces.len() || from == to {
            return;
        }

        let workspace = self.workspaces.remove(from);
        self.workspaces.insert(to, workspace);

        // Update active index to follow the active workspace
        if self.active_workspace_index == from {
            self.active_workspace_index = to;
        } else if from < self.active_workspace_index && to >= self.active_workspace_index {
            self.active_workspace_index -= 1;
        } else if from > self.active_workspace_index && to <= self.active_workspace_index {
            self.active_workspace_index += 1;
        }

        self.serialize(cx);
        cx.notify();
    }

    pub fn open_project(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let old_workspace = self.workspace().clone();
        let old_is_empty = {
            let workspace = old_workspace.read(cx);
            workspace.project().read(cx).worktrees(cx).next().is_none()
                && !workspace.items(cx).any(|item| item.is_dirty(cx))
        };

        let task = old_workspace.update(cx, |workspace, cx| {
            workspace.open_workspace_for_paths(true, paths, window, cx)
        });

        if old_is_empty {
            cx.spawn_in(window, async move |this, cx| {
                task.await?;
                this.update_in(cx, |multi_workspace, window, cx| {
                    if let Some(old_index) = multi_workspace
                        .workspaces()
                        .iter()
                        .position(|w| *w == old_workspace)
                    {
                        if multi_workspace.workspaces().len() > 1 {
                            multi_workspace.remove_workspace(old_index, window, cx);
                        }
                    }
                })
                .ok();
                Ok(())
            })
        } else {
            task
        }
    }
}

impl Render for MultiWorkspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // When the sidebar is open, extract the titlebar from the active workspace
        // and render it at the top level so it spans the full window width.
        let titlebar: Option<AnyView> = if self.sidebar_open {
            let workspace = self.workspace().clone();
            let titlebar = workspace.read(cx).titlebar_item();
            workspace.update(cx, |ws, _| ws.set_skip_titlebar_render(true));
            titlebar
        } else {
            // Ensure all workspaces render their own titlebars when sidebar is closed.
            for ws in &self.workspaces {
                ws.update(cx, |ws, _| ws.set_skip_titlebar_render(false));
            }
            None
        };

        let sidebar: Option<AnyElement> = if self.sidebar_open {
            self.sidebar.as_ref().map(|sidebar_handle| {
                let weak = cx.weak_entity();

                let sidebar_width = sidebar_handle.width(cx);
                let resize_handle = deferred(
                    div()
                        .id("sidebar-resize-handle")
                        .absolute()
                        .right(-SIDEBAR_RESIZE_HANDLE_SIZE / 2.)
                        .top(px(0.))
                        .h_full()
                        .w(SIDEBAR_RESIZE_HANDLE_SIZE)
                        .cursor_col_resize()
                        .on_drag(DraggedSidebar, |dragged, _, _, cx| {
                            cx.stop_propagation();
                            cx.new(|_| dragged.clone())
                        })
                        .on_mouse_down(MouseButton::Left, |_, _, cx| {
                            cx.stop_propagation();
                        })
                        .on_mouse_up(MouseButton::Left, move |event, _, cx| {
                            if event.click_count == 2 {
                                weak.update(cx, |this, cx| {
                                    if let Some(sidebar) = this.sidebar.as_mut() {
                                        sidebar.set_width(None, cx);
                                    }
                                })
                                .ok();
                                cx.stop_propagation();
                            }
                        })
                        .occlude(),
                );

                div()
                    .id("sidebar-container")
                    .relative()
                    .h_full()
                    .w(sidebar_width)
                    .flex_shrink_0()
                    .child(sidebar_handle.to_any())
                    .child(resize_handle)
                    .into_any_element()
            })
        } else {
            None
        };

        let workspace_container = {
            let container = div()
                .flex()
                .flex_1()
                .size_full()
                .overflow_hidden()
                .relative();

            if let Some(previous_index) = self.previous_workspace_index {
                let transition_id = self.transition_id;
                let previous_workspace = self.workspaces[previous_index].clone();
                let active_workspace = self.workspace().clone();

                container
                    .child(
                        div()
                            .id(("crossfade-out", transition_id))
                            .absolute()
                            .size_full()
                            .child(previous_workspace)
                            .with_animation(
                                ("crossfade-out", transition_id),
                                Animation::new(CROSSFADE_DURATION)
                                    .with_easing(ease_out_quint()),
                                |this, delta| this.opacity(1.0 - delta),
                            ),
                    )
                    .child(
                        div()
                            .id(("crossfade-in", transition_id))
                            .size_full()
                            .child(active_workspace)
                            .with_animation(
                                ("crossfade-in", transition_id),
                                Animation::new(CROSSFADE_DURATION)
                                    .with_easing(ease_out_quint()),
                                |this, delta| this.opacity(delta),
                            ),
                    )
            } else {
                container.child(self.workspace().clone())
            }
        };

        let actions_div = div()
            .key_context("Workspace")
            .size_full()
            .on_action(
                cx.listener(|this: &mut Self, _: &NewWorkspaceInWindow, window, cx| {
                    this.create_workspace(window, cx);
                }),
            )
            .on_action(
                cx.listener(|this: &mut Self, _: &NextWorkspaceInWindow, window, cx| {
                    this.activate_next_workspace(window, cx);
                }),
            )
            .on_action(cx.listener(
                |this: &mut Self, _: &PreviousWorkspaceInWindow, window, cx| {
                    this.activate_previous_workspace(window, cx);
                },
            ))
            .on_action(cx.listener(
                |this: &mut Self, _: &RemoveActiveWorkspace, window, cx| {
                    this.remove_active_workspace(window, cx);
                },
            ))
            .on_action(cx.listener(
                |this: &mut Self, _: &ToggleWorkspaceSidebar, window, cx| {
                    this.toggle_sidebar(window, cx);
                },
            ))
            .on_action(
                cx.listener(|this: &mut Self, _: &FocusWorkspaceSidebar, window, cx| {
                    this.focus_sidebar(window, cx);
                }),
            );

        let content = if titlebar.is_some() {
            // Sidebar open: titlebar spans full width at top,
            // sidebar and workspace content sit below it.
            actions_div
                .flex()
                .flex_col()
                .children(titlebar)
                .child(
                    h_flex()
                        .flex_1()
                        .overflow_hidden()
                        .on_drag_move(cx.listener(
                            |this: &mut Self, e: &DragMoveEvent<DraggedSidebar>, _window, cx| {
                                if let Some(sidebar) = &this.sidebar {
                                    let new_width = e.event.position.x;
                                    sidebar.set_width(Some(new_width), cx);
                                }
                            },
                        ))
                        .children(sidebar)
                        .child(workspace_container),
                )
        } else {
            // No sidebar: workspace takes full width with its own titlebar.
            actions_div.child(workspace_container)
        };

        client_side_decorations(
            content,
            window,
            cx,
            Tiling {
                left: self.sidebar_open,
                ..Tiling::default()
            },
        )
    }
}
