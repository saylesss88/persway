//! Stack‑main layout manager for Persway.
//!
//! Implements a two‑region layout:
//! - A “main” area with a fixed relative width.
//! - A “stack” area containing the rest of the windows, laid out as `tabbed`, `stacked`, or tiled.
//!
//! Handles `new`, `close`, `move`, and `floating` window events to maintain this structure.

use crate::{
    layout::StackLayout,
    node_ext::NodeExt,
    utils::{get_focused_workspace, is_persway_tmp_workspace, is_scratchpad_workspace},
};

use anyhow::Result;
use swayipc_async::{Connection, WindowChange, WindowEvent, Workspace};

use super::super::traits::WindowEventHandler;

/// Decide whether a workspace should be skipped for stack‑main layout.
///
/// “Special” workspaces (e.g., temporary or scratchpad) are not managed by stack‑main.
fn should_skip_layout_of_workspace(workspace: &Workspace) -> bool {
    is_persway_tmp_workspace(workspace) || is_scratchpad_workspace(workspace)
}

/// Stack‑main layout manager.
///
/// Maintains:
/// - A fixed‑size main column for the “main” window.
/// - A stack area for the remaining windows, laid out as `tabbed`, `stacked`, or tiled.
/// - Sway‑level layout commands triggered by window events.
pub struct StackMain {
    /// Connection to Sway IPC used for querying the tree and running commands.
    connection: Connection,
    /// Relative size of the main area as a percentage (0–100).
    size: u8,
    /// How the stack area is laid out (`Tabbed`, `Stacked`, or `Tiled`).
    stack_layout: StackLayout,
}

impl StackMain {
    /// Entry point for a stack‑main layout pass.
    ///
    /// Creates a `StackMain` instance with given `size` and `stack_layout`,
    /// and dispatches the `WindowEvent` to the appropriate handler method.
    ///
    /// # Arguments
    /// - `event`: The event to process (wrapped in `Box`).
    /// - `size`: Main area size in percent.
    /// - `stack_layout`: Layout for the stack area (`tabbed` / `stacked` / `tiled`).
    pub async fn handle(event: Box<WindowEvent>, size: u8, stack_layout: StackLayout) {
        if let Ok(mut manager) = Self::new(size, stack_layout).await {
            manager.handle(event).await;
        }
    }

    /// Create a new `StackMain` instance.
    ///
    /// Connects to Sway IPC and initializes internal layout parameters.
    pub async fn new(size: u8, stack_layout: StackLayout) -> Result<Self> {
        let connection = Connection::new().await?;
        Ok(Self {
            connection,
            size,
            stack_layout,
        })
    }

    /// Handle a `WindowChange::New` event for stack‑main layout.
    ///
    /// Adjusts the workspace layout when a new window appears:
    /// - Layout‑1 (1 node): split horizontally and place the new window in main.
    /// - Layout‑2 (2 nodes): mark one node as stack, apply stack layout, and position main.
    /// - Layout‑3 (3+ nodes in stack): reorganize stack using marks and swaps.
    async fn on_new_window(&mut self, event: &WindowEvent) -> Result<()> {
        let tree = self.connection.get_tree().await?;
        let node = tree
            .find_as_ref(|n| n.id == event.container.id)
            .unwrap_or_else(|| panic!("no node found with id {}", event.container.id));
        let ws = node.get_workspace().await?;
        if should_skip_layout_of_workspace(&ws) {
            log::debug!("skip stack_main layout of \"special\" workspace");
            return Ok(());
        }

        if node.is_floating() || node.is_full_screen() {
            log::debug!("skip stack_main layout of \"floating\" \"fullscreen\" workspace");
            return Ok(());
        }

        let wstree = tree.find_as_ref(|n| n.id == ws.id).unwrap();
        log::debug!("new_window id: {}", event.container.id);
        log::debug!("workspace nodes len: {}", wstree.nodes.len());
        let layout = match self.stack_layout {
            StackLayout::Tabbed => "split v; layout tabbed",
            StackLayout::Stacked => "split v; layout stacking",
            StackLayout::Tiled => "split v",
        };
        match wstree.nodes.len() {
            1 => {
                let cmd = format!("[con_id={}] focus; split h", event.container.id);
                self.connection.run_command(cmd).await?;
                Ok(())
            }
            2 => {
                let main = wstree.nodes.last().expect("main window not found");
                let stack = wstree.nodes.first().expect("stack container not found");

                let cmd = if stack.is_window() {
                    format!(
                        "[con_id={}] focus; {}; resize set width {}; [con_id={}] focus",
                        stack.id,
                        layout,
                        (100 - self.size),
                        main.id
                    )
                } else if let Some(node) = stack.find_as_ref(|n| n.id == event.container.id) {
                    format!(
                        "[con_id={}] focus; swap container with con_id {}; [con_id={}] focus",
                        main.id, node.id, node.id
                    )
                } else {
                    String::from("nop event container not in stack")
                };

                self.connection.run_command(cmd).await?;
                Ok(())
            }
            3 => {
                let main = wstree
                    .nodes
                    .iter()
                    .skip(1)
                    .find(|n| n.is_window() && n.id != event.container.id)
                    .expect("main window not found");
                let stack = wstree.nodes.first().expect("stack container not found");
                let stack_mark = format!("_stack_{}", stack.id);

                let cmd = format!(
                    "[con_id={}] mark --add {}; [con_id={}] focus; move container to mark {}; [con_mark={}] unmark {}; [con_id={}] focus; swap container with con_id {}; [con_id={}] focus",
                    stack.id,
                    stack_mark,
                    event.container.id,
                    stack_mark,
                    stack_mark,
                    stack_mark,
                    main.id,
                    event.container.id,
                    event.container.id
                );

                log::debug!("new_window: {cmd}");

                self.connection.run_command(cmd).await?;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Handle a `WindowChange::Close` event for stack‑main layout.
    ///
    /// Adjusts layout when a window is closed, usually by:
    /// - Moving the stack back to `splith` or resizing it if only one window remains.
    async fn on_close_window(&mut self, event: &WindowEvent) -> Result<()> {
        let tree = self.connection.get_tree().await?;
        let ws = get_focused_workspace(&mut self.connection).await?;
        if should_skip_layout_of_workspace(&ws) {
            log::debug!("skip stack_main layout of \"special\" workspace");
            return Ok(());
        }

        let wstree = tree.find_as_ref(|n| n.id == ws.id).unwrap();

        if wstree.nodes.len() == 1
            && let Some(stack) = wstree.nodes.iter().find(|n| n.id != event.container.id)
        {
            let stack_current = stack
                .find_as_ref(|n| n.is_window() && n.focused)
                .unwrap_or_else(|| {
                    stack
                        .find_as_ref(|n| n.visible.unwrap_or(false))
                        .expect("stack should have a visible node")
                });

            let cmd = if wstree.iter().filter(|n| n.is_window()).count() == 1 {
                log::debug!("on_close_window, count 1, stack_id: {}", stack_current.id);
                format!(
                    "[con_id={}] focus; layout splith; move up",
                    stack_current.id
                )
            } else {
                log::debug!(
                    "on_close_window, count more than 1, stack_id: {}",
                    stack_current.id
                );
                format!(
                    "[con_id={}] focus; move right; resize set width {}",
                    stack_current.id, self.size
                )
            };
            log::debug!("close_window: {cmd}");
            self.connection.run_command(cmd).await?;
        }

        Ok(())
    }

    /// Handle a `WindowChange::Move` event for stack‑main layout.
    ///
    /// When a window is moved:
    /// - If it moves within the same workspace, treat it as a new window layout.
    /// - If it moves to another workspace, call `on_new_window` for the target workspace
    ///   and `on_close_window` for the source workspace.
    async fn on_move_window(&mut self, event: &WindowEvent) -> Result<()> {
        let tree = self.connection.get_tree().await?;

        let Some(node) = tree.find_as_ref(|n| n.id == event.container.id) else {
            log::warn!("no node found with id {}", event.container.id);
            return Ok(());
        };

        let Ok(ws) = node.get_workspace().await else {
            log::warn!("node had no workspace");
            return self.on_close_window(event).await;
        };

        if should_skip_layout_of_workspace(&ws) {
            log::debug!("skip stack_main layout of \"special\" workspace");
            return Ok(());
        }

        if node.is_floating() || node.is_full_screen() {
            log::debug!("skip stack_main layout of \"floating\" \"fullscreen\" workspace");
            return Ok(());
        }

        let focused_ws = get_focused_workspace(&mut self.connection).await?;

        if ws.id == focused_ws.id {
            log::debug!("move_window within workspace: {}", ws.num);
            return self.on_new_window(event).await;
        }

        log::debug!("move_window to other workspace: {}", ws.num);
        self.on_new_window(event).await?;
        self.on_close_window(event).await
    }
}

impl WindowEventHandler for StackMain {
    /// Handle a `WindowEvent` in the stack‑main layout manager.
    ///
    /// Dispatches:
    /// - `New` → `on_new_window`.
    /// - `Close` → `on_close_window`.
    /// - `Move` → `on_move_window`.
    /// - `Floating` → `on_close_window` (if floated) or `on_new_window` (if un‑floated).
    ///   Others are logged and ignored.
    async fn handle(&mut self, event: Box<WindowEvent>) {
        match event.change {
            WindowChange::New => {
                log::debug!("stack_main handler handling event: {:?}", event.change);
                if let Err(e) = self.on_new_window(&event).await {
                    log::error!("stack_main layout err: {e}");
                }
            }
            WindowChange::Close => {
                log::debug!("stack_main handler handling event: {:?}", event.change);
                if let Err(e) = self.on_close_window(&event).await {
                    log::error!("stack_main layout err: {e}");
                }
            }
            WindowChange::Move => {
                log::debug!("stack_main handler handling event: {:?}", event.change);
                if let Err(e) = self.on_move_window(&event).await {
                    log::error!("stack_main layout err: {e}");
                }
            }
            WindowChange::Floating => {
                log::debug!("stack_main handler handling event: {:?}", event.change);
                log::debug!(
                    "stack_main is floating: {:?}",
                    event.container.is_floating()
                );
                if event.container.is_floating() {
                    if let Err(e) = self.on_close_window(&event).await {
                        log::error!("stack_main layout err: {e}");
                    }
                } else if let Err(e) = self.on_new_window(&event).await {
                    log::error!("stack_main layout err: {e}");
                }
            }
            _ => {
                log::debug!("stack_main not handling event: {:?}", event.change);
            }
        }
    }
}
