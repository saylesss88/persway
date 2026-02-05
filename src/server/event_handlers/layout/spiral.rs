use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use super::super::traits::WindowEventHandler;
use crate::{
    node_ext::NodeExt,
    utils::{is_persway_tmp_workspace, is_scratchpad_workspace},
};

use anyhow::Result;
use swayipc_async::{Connection, NodeLayout, WindowChange, WindowEvent, Workspace};

pub struct Spiral {
    connection: Connection,
    last_focused_id: Option<i64>,
    last_layout_time: Option<Instant>,
}

fn should_skip_layout_of_workspace(workspace: &Workspace) -> bool {
    is_persway_tmp_workspace(workspace) || is_scratchpad_workspace(workspace)
}

impl Spiral {
    /// Spawns a background task that processes events serially
    /// Returns a sender to send events to the handler
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

    async fn new() -> Result<Self> {
        let connection = Connection::new().await?;
        Ok(Self {
            connection,
            last_focused_id: None,
            last_layout_time: None,
        })
    }

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
