use std::convert::Infallible;
use std::sync::Arc;

use async_stream::stream;
use axum::extract::State;
use axum::response::sse::{Event, Sse};
use axum::Json;
use futures::StreamExt;
use rig_core::agent::MultiTurnStreamItem;
use rig_core::message::Text;
use rig_core::streaming::{StreamedAssistantContent, StreamingPrompt};
use serde::Deserialize;
use tokio_stream::Stream;

use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct ChatRequest {
    pub user_id: String,
    pub question: String,
}

pub async fn handle_chat(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ChatRequest>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let user_id = payload.user_id.clone();
    let question = payload.question;

    let stream = stream! {
        tracing::info!(user_id = %user_id, "request queued, waiting for agent lock");
        let agent = state.agent.lock().await;
        tracing::info!(user_id = %user_id, "agent lock acquired, streaming response");

        let mut rig_stream = agent.stream_prompt(question).await;
        while let Some(item) = rig_stream.next().await {
            match item {
                Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(
                    Text { text },
                ))) => {
                    yield Ok(Event::default().data(text));
                }
                Ok(MultiTurnStreamItem::StreamAssistantItem(
                    StreamedAssistantContent::Reasoning(reasoning),
                )) => {
                    let text = reasoning.display_text();
                    if !text.is_empty() {
                        yield Ok(Event::default().data(text));
                    }
                }
                Ok(MultiTurnStreamItem::FinalResponse(_)) => {}
                Err(err) => {
                    tracing::error!(user_id = %user_id, error = %err, "stream error");
                    yield Ok(Event::default().data(format!("Error: {err}")));
                    break;
                }
                _ => {}
            }
        }

        tracing::info!(user_id = %user_id, "request complete, releasing agent lock");
    };

    Sse::new(stream)
}
