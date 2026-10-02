//! Brique « agent » : serveur MCP minimal sur stdio.
//!
//! Deux outils seulement, pour vérifier que Claude Code voit le serveur et sait l'appeler.

use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::{Json, ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::note;

#[derive(Debug, Clone)]
struct Tracker {
    tool_router: ToolRouter<Self>,
}

#[derive(Deserialize, JsonSchema)]
struct NoteRequest {
    /// Note au format ProTracker, de `C-1` à `B-3` (ex. `A-2`, `C#3`).
    note: String,
}

#[derive(Serialize, JsonSchema)]
struct NoteInfo {
    note: String,
    /// Période Amiga (finetune 0), telle qu'écrite dans un `.mod`.
    period: u32,
    /// Fréquence de lecture du sample sur un Amiga PAL, en Hz.
    sample_rate_hz: f64,
}

#[tool_router(router = tool_router)]
impl Tracker {
    fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "Vérifie que le serveur smpltrckr répond et donne sa version.")]
    async fn ping(&self) -> String {
        format!(
            "smpltrckr {} — phase 0, serveur MCP opérationnel",
            env!("CARGO_PKG_VERSION")
        )
    }

    #[tool(
        description = "Donne la période Amiga et la fréquence de lecture d'une note ProTracker (C-1 à B-3)."
    )]
    async fn note_info(
        &self,
        Parameters(req): Parameters<NoteRequest>,
    ) -> Result<Json<NoteInfo>, String> {
        let index = note::parse(&req.note)
            .ok_or_else(|| format!("note inconnue : {:?} (attendu : C-1 à B-3)", req.note))?;
        let period = note::PERIODS[index];
        Ok(Json(NoteInfo {
            note: note::name(index),
            period: period as u32,
            sample_rate_hz: note::period_to_hz(period),
        }))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Tracker {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("smpltrckr", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "smpltrckr, tracker façon ProTracker. Phase 0 : outils de test uniquement.",
            )
    }
}

pub fn run() -> anyhow::Result<()> {
    tokio::runtime::Runtime::new()?.block_on(async {
        let service = Tracker::new().serve(rmcp::transport::stdio()).await?;
        service.waiting().await?;
        Ok(())
    })
}
