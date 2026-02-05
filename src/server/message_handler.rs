use std::collections::HashMap;

use anyhow::Result;
use swayipc_async::{Connection, WindowEvent};
use tokio::sync::mpsc;
use tokio::task; // Add this import

use super::command_handlers;
use super::event_handlers;
use super::event_handlers::traits::WindowEventHandler;

use crate::server::event_handlers::layout::spiral::Spiral;
use crate::{commands::PerswayCommand, layout::WorkspaceLayout, utils};

#[derive(Debug)]
pub struct WorkspaceConfig {
    layout: WorkspaceLayout,
}

pub struct MessageHandler {
    connection: Connection,
    workspace_config: HashMap<i32, WorkspaceConfig>,
    default_layout: WorkspaceLayout,
    workspace_renaming: bool,
    window_focus_handler: event_handlers::misc::window_focus::WindowFocus,
    spiral_tx: mpsc::UnboundedSender<Box<WindowEvent>>, // Add this field
    rename_handle: Option<task::JoinHandle<()>>,
}

// Remove Debug derive from MessageHandler since mpsc::UnboundedSender doesn't implement Debug
// Or manually implement Debug if you need it

impl MessageHandler {
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

    pub fn get_workspace_config(&mut self, ws_num: i32) -> &WorkspaceConfig {
        self.workspace_config
            .entry(ws_num)
            .or_insert_with(|| WorkspaceConfig {
                layout: self.default_layout.clone(),
            })
    }

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
        // (Note: Removed the duplicate WorkspaceRenamer block that was right here)
        self.window_focus_handler.handle(event).await;

        Ok(())
    }
    pub async fn handle_command(&mut self, cmd: PerswayCommand) -> Result<()> {
        log::debug!("controller.handle_command: {cmd:?}");
        let ws = utils::get_focused_workspace(&mut self.connection).await?;

        let current_ws_config = self.get_workspace_config(ws.num);
        match cmd {
            PerswayCommand::ChangeLayout { layout } => {
                if current_ws_config.layout == layout {
                    log::debug!(
                        "no layout change of ws {} as the requested one was already set",
                        ws.num,
                    );
                } else {
                    self.workspace_config
                        .entry(ws.num)
                        .and_modify(|e| e.layout = layout.clone())
                        .or_insert_with(|| WorkspaceConfig {
                            layout: layout.clone(),
                        });
                    log::debug!("change layout of ws {} to {}", ws.num, layout);
                    log::debug!("start relayout of ws {}", ws.num);

                    task::spawn(utils::relayout_workspace(
                        ws.num,
                        |mut conn, ws_num, _old_ws_id, _output_id, windows| async move {
                            for window in windows.iter().rev() {
                                let cmd = format!(
                                    "[con_id={}] move to workspace number {}; [con_id={}] focus",
                                    window.id, ws_num, window.id
                                );
                                log::debug!("relayout closure cmd: {cmd}");
                                conn.run_command(cmd).await?;
                                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                            }
                            Ok(())
                        },
                    ));
                }
            }
            PerswayCommand::StackFocusNext => {
                if let WorkspaceLayout::StackMain { .. } = current_ws_config.layout {
                    let mut ctrl = command_handlers::layout::stack_main::StackMain::new().await?;
                    ctrl.stack_focus_next().await?;
                }
            }
            PerswayCommand::StackFocusPrev => {
                if let WorkspaceLayout::StackMain { .. } = current_ws_config.layout {
                    let mut ctrl = command_handlers::layout::stack_main::StackMain::new().await?;
                    ctrl.stack_focus_prev().await?;
                }
            }
            PerswayCommand::StackMainRotatePrev => {
                if let WorkspaceLayout::StackMain { .. } = current_ws_config.layout {
                    let mut ctrl = command_handlers::layout::stack_main::StackMain::new().await?;
                    ctrl.stack_main_rotate_prev().await?;
                }
            }
            PerswayCommand::StackMainRotateNext => {
                if let WorkspaceLayout::StackMain { .. } = current_ws_config.layout {
                    let mut ctrl = command_handlers::layout::stack_main::StackMain::new().await?;
                    ctrl.stack_main_rotate_next().await?;
                }
            }
            PerswayCommand::StackSwapMain => {
                if let WorkspaceLayout::StackMain { .. } = current_ws_config.layout {
                    let mut ctrl = command_handlers::layout::stack_main::StackMain::new().await?;
                    ctrl.stack_swap_main().await?;
                }
            }
            PerswayCommand::Daemon(_) => unreachable!(),
        }
        Ok(())
    }
}
