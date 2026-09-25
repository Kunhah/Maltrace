//! Claude-powered investigative analyst panel.
//!
//! At startup we read the two Claude-OSINT skills (`osint-methodology` and
//! `offensive-osint`) from disk into memory. Each chat turn sends them as
//! the system prompt with a `cache_control: ephemeral` breakpoint on the
//! last block — the prefix is large (~235 KB combined) so prompt caching
//! cuts repeat-turn cost ~90% after the first turn.
//!
//! Graph context (entity counts, selected node, recent entities) is sent in
//! the *user* message, never in `system`. Volatile content after the cache
//! breakpoint is the rule: putting graph state in system would invalidate
//! the cache on every selection change.
//!
//! API key reads from `ANTHROPIC_API_KEY` at process startup. Missing key →
//! the chat function returns a friendly error string rather than panicking;
//! the UI surfaces it as a toast.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

/// Default model. Claude Opus 4.7 — the strongest model, worth it for an
/// analyst that has to reason over investigation context. Override via env
/// `MALTRACE_AI_MODEL`.
const DEFAULT_MODEL: &str = "claude-opus-4-7";
const DEFAULT_MAX_TOKENS: u32 = 4096;
const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Debug, Clone)]
pub struct AiState {
    pub api_key: Option<String>,
    pub model: String,
    pub max_tokens: u32,
    /// Concatenated, ready-to-send skill text. Stable across the lifetime of
    /// the process, which is exactly what the cache wants.
    pub system_prompt: Arc<String>,
    pub skills_loaded: bool,
}

impl AiState {
    /// Build the AI state at startup. Reading skills is best-effort: if the
    /// files aren't present, the panel still works (with a generic system
    /// prompt) and we log a warning rather than failing the whole app boot.
    pub fn load() -> Self {
        let api_key = std::env::var("ANTHROPIC_API_KEY").ok().filter(|k| !k.is_empty());
        let model = std::env::var("MALTRACE_AI_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
        let max_tokens = std::env::var("MALTRACE_AI_MAX_TOKENS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_MAX_TOKENS);

        let (system_prompt, skills_loaded) = match load_skills() {
            Ok(text) => (Arc::new(text), true),
            Err(e) => {
                eprintln!(
                    "maltrace ai: skills not loaded ({e}); the panel will use a generic prompt."
                );
                (Arc::new(generic_prompt()), false)
            }
        };

        Self { api_key, model, max_tokens, system_prompt, skills_loaded }
    }
}

/// Find and read the Claude-OSINT skill files. Looks for the sibling
/// checkout next to `maltrace/` first; honors `MALTRACE_CLAUDE_OSINT_PATH`
/// when set.
fn load_skills() -> Result<String> {
    let root = resolve_claude_osint_path()?;
    let methodology = root.join("skills/osint-methodology/SKILL.md");
    let offensive = root.join("skills/offensive-osint/SKILL.md");

    let mut out = String::with_capacity(256 * 1024);
    out.push_str(PREAMBLE);
    out.push_str("\n\n---\n\n# Skill: OSINT methodology\n\n");
    out.push_str(
        &std::fs::read_to_string(&methodology)
            .with_context(|| format!("reading {}", methodology.display()))?,
    );
    out.push_str("\n\n---\n\n# Skill: Offensive OSINT\n\n");
    out.push_str(
        &std::fs::read_to_string(&offensive)
            .with_context(|| format!("reading {}", offensive.display()))?,
    );
    Ok(out)
}

fn resolve_claude_osint_path() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("MALTRACE_CLAUDE_OSINT_PATH") {
        let path = PathBuf::from(p);
        if path.join("skills").exists() {
            return Ok(path);
        }
    }
    // Layout: <root>/maltrace/app/src-tauri/  <- CARGO_MANIFEST_DIR
    //         <root>/Claude-OSINT/
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for candidate in [
        manifest_dir.join("..").join("..").join("..").join("Claude-OSINT"),
        manifest_dir.join("..").join("..").join("Claude-OSINT"),
    ] {
        if candidate.join("skills").exists() {
            return Ok(candidate);
        }
    }
    Err(anyhow!(
        "could not locate Claude-OSINT; set MALTRACE_CLAUDE_OSINT_PATH or place the checkout as a sibling to maltrace/"
    ))
}

const PREAMBLE: &str = "\
You are the investigative analyst panel inside maltrace, an open-source \
graph-based OSINT tool. The user is conducting an investigation — they \
add entities (domains, emails, people, IPs, images), run transforms that \
pivot them to related entities, and watch the graph grow.

Your job is to help the investigator: summarize what's on the graph, \
suggest the next pivot, explain what an entity or transform output means, \
and point at gaps. Reference the two skills below (OSINT methodology + \
offensive OSINT tradecraft) when relevant.

When the user's message starts with a `[Graph context: ...]` block, the \
remainder is the actual question — use the block to ground your answer in \
specifics from their investigation. Be concrete and concise: an answer \
the investigator can act on in one step is better than a long lecture.

Do not invent entities, edges, or transforms that aren't in the graph or \
catalogue. If you're unsure, say so and propose a transform to run.";

fn generic_prompt() -> String {
    PREAMBLE.to_string()
}

// ──────────────────────────────────────────────────────────────────────
//  Public API: chat()
// ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatTurn {
    pub role: String, // "user" | "assistant"
    pub content: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatInput {
    pub history: Vec<ChatTurn>,
    pub message: String,
    /// Optional graph summary prepended to the user message as
    /// `[Graph context: …]`. Computed by the caller (see core.rs) so we
    /// stay transport-agnostic.
    pub graph_context: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatReply {
    pub message: String,
    pub model: String,
    pub usage: Usage,
    pub skills_loaded: bool,
    /// Tool calls Claude made during this turn, in the order it made
    /// them — surfaced to the UI so each "running sfp_whois on
    /// example.com" hop appears inline in the chat transcript. Empty
    /// when the user disabled tools or Claude didn't pivot.
    #[serde(default)]
    pub tool_calls: Vec<ToolCallTurn>,
}

/// One executed tool invocation. Carried alongside the assistant's
/// final reply so the UI can render Claude's investigation path.
#[derive(Debug, Clone, Serialize)]
pub struct ToolCallTurn {
    pub tool_name: String,
    /// JSON-stringified args (small; the tool inputs are tight schemas).
    pub args: String,
    /// JSON-stringified result handed back to Claude. Truncated to a
    /// few KB so a noisy tool can't blow up the chat panel.
    pub result: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
}

/// Raw outcome of one Anthropic request. `text` is the concatenated
/// text blocks; `tool_uses` is each `tool_use` block Claude emitted
/// (used by the core loop to execute tools and re-call until Claude
/// stops requesting them). `stop_reason` lets the loop detect the
/// natural end of a turn (`end_turn`) vs more tool calls coming
/// (`tool_use`).
#[derive(Debug, Clone)]
pub struct RawResponse {
    pub model: String,
    pub usage: Usage,
    /// Every content block, preserved verbatim so the loop can replay
    /// it as the next round's assistant turn.
    pub raw_content: Vec<serde_json::Value>,
    pub text: String,
    pub tool_uses: Vec<ToolUseRequest>,
    pub stop_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ToolUseRequest {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

/// One round-trip to the Anthropic API. The caller composes the
/// `messages` array (system prompt + history + tool turns); we issue
/// the request, surface text + tool_use blocks. The tool-use loop
/// lives in `core::ai_chat` where the AppState's read/write access to
/// the graph + sidecars makes tool execution natural.
pub async fn chat_once(
    state: &AiState,
    messages: &[serde_json::Value],
    tools: Option<&[serde_json::Value]>,
) -> Result<RawResponse> {
    let api_key = state.api_key.as_ref().ok_or_else(|| {
        anyhow!(
            "ANTHROPIC_API_KEY is not set. Launch maltrace with `ANTHROPIC_API_KEY=sk-ant-… npm run tauri dev`."
        )
    })?;

    let system = serde_json::json!([
        {
            "type": "text",
            "text": *state.system_prompt,
            "cache_control": {"type": "ephemeral"},
        }
    ]);

    let mut body = serde_json::json!({
        "model": state.model,
        "max_tokens": state.max_tokens,
        "thinking": {"type": "adaptive"},
        "system": system,
        "messages": messages,
    });
    if let Some(tool_specs) = tools {
        if !tool_specs.is_empty() {
            body["tools"] = serde_json::Value::Array(tool_specs.to_vec());
        }
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .context("building HTTP client")?;

    let resp = client
        .post(ANTHROPIC_URL)
        .header("x-api-key", api_key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .context("calling Anthropic API")?;

    let status = resp.status();
    let raw = resp.text().await.context("reading Anthropic response body")?;

    if !status.is_success() {
        let pretty = serde_json::from_str::<serde_json::Value>(&raw)
            .ok()
            .and_then(|v| v.get("error").and_then(|e| e.get("message")).cloned())
            .and_then(|m| m.as_str().map(String::from))
            .unwrap_or_else(|| raw.chars().take(500).collect());
        return Err(anyhow!("Anthropic API {}: {}", status, pretty));
    }

    let parsed: AnthropicResponse =
        serde_json::from_str(&raw).context("parsing Anthropic response")?;
    // Also parse the raw value so we can hold onto each content block
    // verbatim — the tool-use round-trip needs to echo the original
    // `tool_use` blocks back as the assistant turn unchanged.
    let raw_v: serde_json::Value = serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);
    let raw_content = raw_v
        .get("content")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default();

    let mut text = String::new();
    let mut tool_uses = Vec::new();
    for block in &parsed.content {
        match block {
            ContentBlock::Text { text: t } => text.push_str(t),
            ContentBlock::ToolUse { id, name, input } => {
                tool_uses.push(ToolUseRequest {
                    id: id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                });
            }
            ContentBlock::Thinking { .. } | ContentBlock::Other => {}
        }
    }

    Ok(RawResponse {
        model: parsed.model,
        usage: parsed.usage,
        raw_content,
        text,
        tool_uses,
        stop_reason: parsed.stop_reason,
    })
}

/// Backward-compatible wrapper for callers that don't want tool-use.
/// New code paths should call `chat_once` directly and orchestrate
/// the tool loop themselves.
pub async fn chat(state: &AiState, input: ChatInput) -> Result<ChatReply> {
    let mut user_text = String::new();
    if let Some(ctx) = input.graph_context.as_deref().filter(|s| !s.is_empty()) {
        user_text.push_str("[Graph context: ");
        user_text.push_str(ctx);
        user_text.push_str("]\n\n");
    }
    user_text.push_str(&input.message);

    let mut messages: Vec<serde_json::Value> = input
        .history
        .iter()
        .map(|t| {
            serde_json::json!({
                "role": t.role,
                "content": t.content,
            })
        })
        .collect();
    messages.push(serde_json::json!({
        "role": "user",
        "content": user_text,
    }));

    let resp = chat_once(state, &messages, None).await?;
    Ok(ChatReply {
        message: if resp.text.is_empty() {
            "(empty response from model)".to_string()
        } else {
            resp.text
        },
        model: resp.model,
        usage: resp.usage,
        skills_loaded: state.skills_loaded,
        tool_calls: Vec::new(),
    })
}

/// Default instruction used when the caller doesn't supply one. Tuned for
/// OSINT image triage — geolocation cues, text/OCR, identifying marks, and
/// safety-relevant observations — rather than a generic caption.
pub const DEFAULT_IMAGE_PROMPT: &str = "You are an OSINT image analyst. Examine this image and report, as concise bullet points: \
(1) a one-line summary of what it shows; \
(2) any readable text, logos, signage, license plates, or watermarks (transcribe verbatim); \
(3) geolocation cues — landmarks, architecture, language, vegetation, road signs, climate; \
(4) people/objects of note (no identity guesses, just observable attributes); \
(5) timestamp/EXIF-style cues visible in the frame; \
(6) suggested follow-up OSINT steps. Be factual; flag uncertainty rather than speculating.";

/// Vision call: send one base64 image (plus an instruction) to Claude and
/// return the analysis text. `media_type` must be an Anthropic-supported
/// image mime (image/jpeg|png|gif|webp). `image_b64` is RAW base64 with no
/// `data:` prefix.
pub async fn analyze_image(
    state: &AiState,
    image_b64: &str,
    media_type: &str,
    instruction: &str,
    context: Option<&str>,
) -> Result<ChatReply> {
    let mut text = String::new();
    if let Some(ctx) = context.filter(|s| !s.is_empty()) {
        text.push_str("[Investigation context: ");
        text.push_str(ctx);
        text.push_str("]\n\n");
    }
    text.push_str(instruction);

    let messages = vec![serde_json::json!({
        "role": "user",
        "content": [
            {
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": media_type,
                    "data": image_b64,
                },
            },
            { "type": "text", "text": text },
        ],
    })];

    let resp = chat_once(state, &messages, None).await?;
    Ok(ChatReply {
        message: if resp.text.is_empty() {
            "(empty response from model)".to_string()
        } else {
            resp.text
        },
        model: resp.model,
        usage: resp.usage,
        skills_loaded: state.skills_loaded,
        tool_calls: Vec::new(),
    })
}

// ──────────────────────────────────────────────────────────────────────
//  Anthropic API response shape
// ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    model: String,
    #[serde(default)]
    content: Vec<ContentBlock>,
    #[serde(default)]
    usage: Usage,
    /// `"end_turn"` when Claude is done; `"tool_use"` when it expects
    /// to be re-called after tool_result. The tool-use loop branches
    /// on this.
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "thinking")]
    Thinking {
        #[allow(dead_code)]
        #[serde(default)]
        thinking: String,
    },
    /// Tool-use block — Claude is asking the host to run a tool and
    /// hand the result back as a `tool_result` message before
    /// continuing. We surface these via `RawResponse::tool_uses` so
    /// the loop in `core::ai_chat` can dispatch.
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        #[serde(default)]
        input: serde_json::Value,
    },
    /// Tolerate future block types without breaking parse.
    #[serde(other)]
    Other,
}

// Deserialize support for Usage (needed because the API also emits these
// fields). The Serialize derive on `Usage` is what the frontend sees.
impl<'de> Deserialize<'de> for Usage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(default)]
            input_tokens: u64,
            #[serde(default)]
            output_tokens: u64,
            #[serde(default)]
            cache_creation_input_tokens: u64,
            #[serde(default)]
            cache_read_input_tokens: u64,
        }
        let r = Raw::deserialize(deserializer)?;
        Ok(Usage {
            input_tokens: r.input_tokens,
            output_tokens: r.output_tokens,
            cache_creation_input_tokens: r.cache_creation_input_tokens,
            cache_read_input_tokens: r.cache_read_input_tokens,
        })
    }
}
