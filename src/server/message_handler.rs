//! Persway message handler module.
//!
//! Coordinates:
//! - Workspace‑level layout state (`WorkspaceConfig`).
//! - Event dispatch to layout handlers (`Spiral`, `StackMain`) and `WindowFocus`.
//! - Command handling for `PerswayCommand` such as layout changes and stack commands.

use std::collections::HashMap;

use anyhow::{Result, bail, ensure};
use swayipc_async::{Connection, WindowEvent};
use tokio::sync::mpsc;
use tokio::task;

use super::command_handlers;
use super::event_handlers;
use super::event_handlers::traits::WindowEventHandler;

use crate::server::event_handlers::layout::spiral::Spiral;
use crate::{commands::PerswayCommand, layout::WorkspaceLayout, utils};

/// Configuration associated with a single workspace.
///
/// This struct holds the layout policy for one workspace (e.g., `spiral`, `stack_main`, `manual`).
#[derive(Debug)]
pub struct WorkspaceConfig {
    layout: WorkspaceLayout,
}

/// Main handler for all Sway events and `persway` commands.
///
/// Stores:
/// - Per‑workspace `WorkspaceConfig`s mapped by workspace number.
/// - The default layout for new workspaces.
/// - A Sway `Connection` used for executing layout and rename commands.
/// - A `WindowFocus` handler for opacity/mark‑based focus hooks.
/// - A `mpsc::UnboundedSender` for forwarding events to the `Spiral` layout handler.
/// - Optional `JoinHandle` for debounced workspace renaming.
pub struct MessageHandler {
    connection: Connection,
    workspace_config: HashMap<i32, WorkspaceConfig>,
    default_layout: WorkspaceLayout,
    workspace_renaming: bool,
    window_focus_handler: event_handlers::misc::window_focus::WindowFocus,
    spiral_tx: mpsc::UnboundedSender<Box<WindowEvent>>, // Sender to the Spiral event handler
    rename_handle: Option<task::JoinHandle<()>>,
}

impl MessageHandler {
    /// Create a new `MessageHandler` with default layout and focus hooks.
    ///
    /// # Arguments
    /// - `default_layout`: Layout used for workspaces that haven’t been explicitly configured.
    /// - `workspace_renaming`: If `true`, workspace names are updated based on running apps.
    /// - `on_window_focus`: Optional Sway command run when a window gains focus.
    /// - `on_window_focus_leave`: Optional Sway command run when focus leaves a window.
    pub async fn new(
        default_layout: WorkspaceLayout,
        workspace_renaming: bool,
        on_window_focus: Option<String>,
        on_window_focus_leave: Option<String>,
    ) -> Result<Self> {
        let window_focus_handler = event_handlers::misc::window_focus::WindowFocus::new(
            on_window_focus,
            on_window_focus_leave,
        )
        .await?;

        let connection = Connection::new().await?;

        // Initialize the spiral handler once
        let spiral_tx = Spiral::spawn_handler();

        Ok(Self {
            connection,
            workspace_config: HashMap::new(),
            default_layout,
            workspace_renaming,
            window_focus_handler,
            spiral_tx, // Store it
            rename_handle: None,
        })
    }

    /// Return a mutable reference to the configuration of workspace `ws_num`.
    ///
    /// If no config exists for `ws_num`, a new entry is inserted with `self.default_layout`.
    pub fn get_workspace_config(&mut self, ws_num: i32) -> &WorkspaceConfig {
        self.workspace_config
            .entry(ws_num)
            .or_insert_with(|| WorkspaceConfig {
                layout: self.default_layout.clone(),
            })
    }

    /// Handle a Sway `WindowEvent` by:
    /// 1. Debouncing workspace renaming (if enabled).
    /// 2. Routing the event to the appropriate layout handler (`spiral` or `stack_main`).
    /// 3. Passing the event to the `WindowFocus` handler for opacity/mark effects.
    ///
    /// This method is called from the `Daemon`’s event loop for every `Window` event.
    pub async fn handle_event(&mut self, event: Box<WindowEvent>) -> Result<()> {
        log::debug!("controller.handle_event: {:?}", event.change);

        let ws = utils::get_focused_workspace(&mut self.connection).await?;

        // --- 1. DEBOUNCED RENAMING ---
        if self.workspace_renaming {
            // Cancel the previous pending rename task if it exists
            if let Some(handle) = self.rename_handle.take() {
                handle.abort();
            }

            let event_clone = event.clone();

            // Spawn a new task with a delay
            self.rename_handle = Some(task::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                event_handlers::misc::workspace_renamer::WorkspaceRenamer::handle(event_clone)
                    .await;
            }));
        }

        // --- 2. LAYOUT MANAGEMENT ---
        match &self.get_workspace_config(ws.num).layout {
            WorkspaceLayout::Spiral => {
                log::debug!("handling event via spiral manager");
                if let Err(e) = self.spiral_tx.send(event.clone()) {
                    log::error!("failed to send event to spiral handler: {e}");
                }
            }
            WorkspaceLayout::StackMain { stack_layout, size } => {
                log::debug!("handling event via stack_main manager");
                task::spawn(event_handlers::layout::stack_main::StackMain::handle(
                    event.clone(),
                    *size,
                    stack_layout.clone(),
                ));
            }
            WorkspaceLayout::Manual => {}
        }

        // --- 3. FOCUS HANDLER ---
        self.window_focus_handler.handle(event).await;

        Ok(())
    }

    fn require_stack_main(
        ws_num: i32,
        ws_name: &str,
        layout: &WorkspaceLayout,
        cmd: &str,
    ) -> Result<()> {
        ensure!(
            matches!(layout, WorkspaceLayout::StackMain { .. }),
            "{cmd} only works on stack-main workspaces.\n\
             Focused workspace: {ws_num} ('{ws_name}')\n\
             Current layout: {layout:?}\n\
             Fix: persway change-layout stack-main"
        );
        Ok(())
    }
    /// Handle a `PerswayCommand` such as layout changes or stack commands.
    ///
    /// # Arguments
    /// - `cmd`: The parsed command (e.g., `ChangeLayout`, `StackFocusNext`, etc.).
    ///
    /// The handler:
    /// - Fetches the focused workspace.
    /// - Updates layout state for that workspace if needed.
    /// - Executes the corresponding layout logic asynchronously (e.g., `relayout_workspace`).
    pub async fn handle_command(&mut self, cmd: PerswayCommand) -> Result<()> {
        log::debug!("controller.handle_command: {cmd:?}");
        let ws = utils::get_focused_workspace(&mut self.connection).await?;

        if ws.num < 0 {
            bail!(
                "Focused workspace '{}' has no numeric workspace number, so persway commands that key off ws.num won't apply. \
Consider naming workspaces with a leading number (e.g. '1: web').",
                ws.name
            );
        }

        // Snapshot current layout so we don't keep borrowing self.workspace_config
        let current_layout = self.get_workspace_config(ws.num).layout.clone();

        match cmd {
            PerswayCommand::ChangeLayout { layout } => {
                if current_layout == layout {
                    // Optional: return Ok(()) or print a message; no need to error
                    log::debug!("layout already set for ws {}", ws.num);
                    return Ok(());
                }

                self.workspace_config
                    .entry(ws.num)
                    .and_modify(|e| e.layout = layout.clone())
                    .or_insert_with(|| WorkspaceConfig {
                        layout: layout.clone(),
                    });

                task::spawn(utils::relayout_workspace(
                    ws.num,
                    |mut conn, ws_num, _old_ws_id, _output_id, windows| async move {
                        for window in windows.iter().rev() {
                            let cmd = format!(
                                "[con_id={}] move to workspace number {}; [con_id={}] focus",
                                window.id, ws_num, window.id
                            );
                            conn.run_command(cmd).await?;
                            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        }
                        Ok(())
                    },
                ));
            }

            PerswayCommand::StackFocusNext => {
                Self::require_stack_main(ws.num, &ws.name, &current_layout, "stack-focus-next")?;
                let mut ctrl = command_handlers::layout::stack_main::StackMain::new().await?;
                ctrl.stack_focus_next().await?;
            }

            PerswayCommand::StackFocusPrev => {
                Self::require_stack_main(ws.num, &ws.name, &current_layout, "stack-focus-prev")?;
                let mut ctrl = command_handlers::layout::stack_main::StackMain::new().await?;
                ctrl.stack_focus_prev().await?;
            }

            PerswayCommand::StackMainRotatePrev => {
                Self::require_stack_main(
                    ws.num,
                    &ws.name,
                    &current_layout,
                    "stack-main-rotate-prev",
                )?;
                let mut ctrl = command_handlers::layout::stack_main::StackMain::new().await?;
                ctrl.stack_main_rotate_prev().await?;
            }

            PerswayCommand::StackMainRotateNext => {
                Self::require_stack_main(
                    ws.num,
                    &ws.name,
                    &current_layout,
                    "stack-main-rotate-next",
                )?;
                let mut ctrl = command_handlers::layout::stack_main::StackMain::new().await?;
                ctrl.stack_main_rotate_next().await?;
            }

            PerswayCommand::StackSwapMain => {
                Self::require_stack_main(ws.num, &ws.name, &current_layout, "stack-swap-main")?;
                let mut ctrl = command_handlers::layout::stack_main::StackMain::new().await?;
                ctrl.stack_swap_main().await?;
            }

            PerswayCommand::Daemon(_) => unreachable!(),
        }

        Ok(())
    }
}
