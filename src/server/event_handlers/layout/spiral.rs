//! Spiral layout manager for Persway.
//!
//! Handles:
//! - A background task that serially processes `WindowEvent`s.
//! - Dynamic layout switching (`split v` / `split h`) based on window aspect ratio.
//! - Throttling of rapid focus events to avoid flickering.

use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use super::super::traits::WindowEventHandler;
use crate::{
    node_ext::NodeExt,
    utils::{is_persway_tmp_workspace, is_scratchpad_workspace},
};

use anyhow::Result;
use swayipc_async::{Connection, NodeLayout, WindowChange, WindowEvent, Workspace};

/// Spiral layout manager.
///
/// Runs in a background task and:
/// - Receives `WindowEvent`s via `spiral_tx`.
/// - Calculates whether a node should be `split v` or `split h`.
/// - Applies layout changes via Sway IPC.
/// - Throttles repeated focus events and skips "special" workspaces.
pub struct Spiral {
    /// Connection to Sway used for querying the tree and running commands.
    connection: Connection,
    /// Last focused container ID, used to avoid redundant layout changes.
    last_focused_id: Option<i64>,
    /// Last time a layout pass was performed, used for throttling.
    last_layout_time: Option<Instant>,
}

/// Determine whether a workspace should be skipped for spiral layout.
///
/// Special workspaces (e.g., temporary or scratchpad) are not laid out by spiral.
fn should_skip_layout_of_workspace(workspace: &Workspace) -> bool {
    is_persway_tmp_workspace(workspace) || is_scratchpad_workspace(workspace)
}

impl Spiral {
    /// Spawn a background task that sequentially handles spiral layout events.
    ///
    /// The returned `UnboundedSender` should be used to send `Box<WindowEvent>`
    /// to the spiral manager from the `MessageHandler`.
    ///
    /// # Return
    /// `mpsc::UnboundedSender<Box<WindowEvent>>` for forwarding events to spiral.
    pub fn spawn_handler() -> mpsc::UnboundedSender<Box<WindowEvent>> {
        let (tx, mut rx) = mpsc::unbounded_channel();

        tokio::spawn(async move {
            match Self::new().await {
                Ok(mut manager) => {
                    log::debug!("spiral manager: handler task started");
                    while let Some(event) = rx.recv().await {
                        manager.handle(event).await;
                    }
                    log::debug!("spiral manager: handler task stopped");
                }
                Err(e) => {
                    log::error!("spiral manager: failed to initialize: {e}");
                }
            }
        });

        tx
    }

    /// Create a new `Spiral` instance.
    ///
    /// Connects to Sway IPC and initializes internal state.
    async fn new() -> Result<Self> {
        let connection = Connection::new().await?;
        Ok(Self {
            connection,
            last_focused_id: None,
            last_layout_time: None,
        })
    }

    /// Perform spiral layout for a single window event.
    ///
    /// This method:
    /// - Throttles very rapid layout passes.
    /// - Skips duplicate focus events for the same container.
    /// - Skips special workspaces (tmp, scratchpad).
    /// - Computes whether a node should be `split v` or `split h` and applies it if needed.
    async fn layout(&mut self, event: WindowEvent) -> Result<()> {
        log::debug!("spiral manager handling event: {:?}", event.change);

        if let Some(last_time) = self.last_layout_time
            && last_time.elapsed() < Duration::from_millis(50)
        {
            log::debug!("spiral layout: throttling rapid events");
            return Ok(());
        }

        self.last_layout_time = Some(Instant::now());

        // Check for duplicate focus events
        if self.last_focused_id == Some(event.container.id) {
            log::debug!(
                "spiral layout: duplicate focus event for {}, skipping",
                event.container.id
            );
            return Ok(());
        }
        self.last_focused_id = Some(event.container.id);

        let tree = self.connection.get_tree().await?;

        // Handle stale node references gracefully
        let Some(node) = tree.find_as_ref(|n| n.id == event.container.id) else {
            log::debug!(
                "spiral layout: node {} no longer exists (stale event), skipping",
                event.container.id
            );
            return Ok(());
        };

        let ws = match node.get_workspace().await {
            Ok(ws) => ws,
            Err(e) => {
                log::debug!(
                    "spiral layout: couldn't get workspace for node {} ({}), skipping",
                    node.id,
                    e
                );
                return Ok(());
            }
        };

        if should_skip_layout_of_workspace(&ws) {
            log::debug!("skip spiral layout of \"special\" workspace");
            return Ok(());
        }

        if !(node.is_floating_window()
            || node.is_floating_container()
            || node.is_full_screen()
            || node.is_stacked().await?
            || node.is_tabbed().await?)
        {
            let desired_layout = if node.rect.height > node.rect.width {
                NodeLayout::SplitV
            } else {
                NodeLayout::SplitH
            };

            // ONLY run the command if the current layout is different
            if node.layout == desired_layout {
                log::debug!(
                    "spiral layout: node {} already has correct split, skipping",
                    node.id
                );
            } else {
                let cmd = match desired_layout {
                    NodeLayout::SplitV => format!("[con_id={}] split v", node.id),
                    NodeLayout::SplitH => format!("[con_id={}] split h", node.id),
                    _ => unreachable!(),
                };
                log::debug!("spiral layout: applying change -> {cmd}");
                self.connection.run_command(cmd).await?;
            }
        }

        Ok(())
    }
}

impl WindowEventHandler for Spiral {
    /// Handle a `WindowEvent` in the spiral layout manager.
    ///
    /// Only `WindowChange::Focus` events trigger layout work; all others are logged and ignored.
    async fn handle(&mut self, event: Box<WindowEvent>) {
        match event.change {
            WindowChange::Focus => {
                if let Err(e) = self.layout(*event).await {
                    log::error!("spiral manager, layout err: {e}");
                }
            }
            _ => log::debug!("spiral manager, not handling event: {:?}", event.change),
        }
    }
}
