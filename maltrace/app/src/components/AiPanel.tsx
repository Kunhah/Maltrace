import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { api } from "../api";
import type { AiStatus, ChatTurn, Entity, ToolCallTurn } from "../types";

interface Props {
  caseId: number | null;
  selectedEntity: Entity | null;
  onClose: () => void;
}

/// Floating Claude-powered analyst panel.
///
/// Backend (`ai.rs`) loads the Claude-OSINT skills once at startup and sends
/// them as a cached system prompt (`cache_control: ephemeral`). Each turn
/// after the first should report `cache_read_input_tokens > 0` — that's how
/// we know caching is actually working. The panel shows token usage under
/// each assistant message so the user can sanity-check.
///
/// Conversation history is ephemeral (in component state); we don't persist
/// it to SQLite for the walking skeleton. The graph context is recomputed
/// on every send and prepended to the user's message as `[Graph context: …]`
/// by the backend.
export function AiPanel({ caseId, selectedEntity, onClose }: Props) {
  const [history, setHistory] = useState<ChatTurn[]>([]);
  /// Per-assistant-turn tool calls, indexed by position in `history`.
  /// Stored separately so the wire `ChatTurn` (which the backend
  /// expects) stays role+content only.
  const [toolCallsByTurn, setToolCallsByTurn] = useState<
    Record<number, ToolCallTurn[]>
  >({});
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [includeGraph, setIncludeGraph] = useState(true);
  /// Tool use is opt-in per session — once enabled the user explicitly
  /// authorises Claude to run transforms / drop entities on the
  /// canvas. Default off so a "Hello, what's on the graph?" doesn't
  /// blow the user's API key on side-effect calls.
  const [allowTools, setAllowTools] = useState(false);
  const [status, setStatus] = useState<AiStatus | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const inputRef = useRef<HTMLTextAreaElement | null>(null);
  const [lastUsage, setLastUsage] = useState<{
    in: number;
    out: number;
    cache_read: number;
    cache_write: number;
  } | null>(null);

  useEffect(() => {
    let cancelled = false;
    api
      .aiStatus()
      .then((s) => {
        if (!cancelled) setStatus(s);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // Focus input on mount; scroll on history change.
  useEffect(() => {
    inputRef.current?.focus();
  }, []);
  useEffect(() => {
    scrollRef.current?.scrollTo({
      top: scrollRef.current.scrollHeight,
      behavior: "smooth",
    });
  }, [history, busy]);

  const send = useCallback(async () => {
    if (!input.trim() || !caseId || busy) return;
    const text = input.trim();
    setInput("");
    setError(null);

    const newHistory: ChatTurn[] = [
      ...history,
      { role: "user", content: text },
    ];
    setHistory(newHistory);
    setBusy(true);
    try {
      const reply = await api.aiChat({
        case_id: caseId,
        history,
        message: text,
        include_graph: includeGraph,
        selected_entity_id: selectedEntity?.id ?? null,
        allow_tools: allowTools,
      });
      const assistantIndex = newHistory.length; // position in next history
      setHistory([
        ...newHistory,
        { role: "assistant", content: reply.message },
      ]);
      if (reply.tool_calls && reply.tool_calls.length > 0) {
        setToolCallsByTurn((cur) => ({
          ...cur,
          [assistantIndex]: reply.tool_calls!,
        }));
      }
      setLastUsage({
        in: reply.usage.input_tokens,
        out: reply.usage.output_tokens,
        cache_read: reply.usage.cache_read_input_tokens,
        cache_write: reply.usage.cache_creation_input_tokens,
      });
    } catch (e) {
      setError(String(e));
      // Drop the optimistic user turn we just appended — let them retry.
      setHistory(history);
    } finally {
      setBusy(false);
      inputRef.current?.focus();
    }
  }, [input, history, caseId, busy, includeGraph, selectedEntity, allowTools]);

  const onKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        send();
      } else if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    },
    [send, onClose],
  );

  const clear = useCallback(() => {
    setHistory([]);
    setToolCallsByTurn({});
    setError(null);
    setLastUsage(null);
  }, []);

  const cacheHitPct = useMemo(() => {
    if (!lastUsage) return null;
    const total = lastUsage.in + lastUsage.cache_read + lastUsage.cache_write;
    if (total === 0) return null;
    return Math.round((lastUsage.cache_read / total) * 100);
  }, [lastUsage]);

  return (
    <aside
      className="ai-panel"
      role="complementary"
      aria-label="AI analyst chat"
    >
      <header className="ai-panel-header">
        <span className="ai-panel-mark" aria-hidden="true">
          ✦
        </span>
        <span className="ai-panel-title">Analyst</span>
        {status?.model && (
          <span className="ai-panel-model" title={status.model}>
            {status.model.replace(/^claude-/, "")}
          </span>
        )}
        {status && !status.skills_loaded && (
          <span
            className="ai-panel-warn"
            title="Claude-OSINT skill files not found; running with a generic prompt only"
          >
            skills off
          </span>
        )}
        <button
          className="icon-btn"
          onClick={clear}
          disabled={history.length === 0 || busy}
          title="Clear conversation"
          aria-label="Clear conversation"
        >
          ⟲
        </button>
        <button
          className="icon-btn"
          onClick={onClose}
          title="Close (Esc)"
          aria-label="Close AI panel"
        >
          ×
        </button>
      </header>

      <div className="ai-panel-messages" ref={scrollRef}>
        {history.length === 0 && (
          <div className="ai-panel-empty">
            <p>
              Ask anything about your investigation. Suggestions: "What pivots
              should I run on the selected entity?", "Summarize what we've
              found so far", "What does this WHOIS tell us?"
            </p>
            {status && !status.configured && (
              <p className="ai-panel-warn-block">
                <strong>ANTHROPIC_API_KEY is not set.</strong> Restart the app
                with{" "}
                <code>ANTHROPIC_API_KEY=sk-ant-… npm run tauri dev</code>.
              </p>
            )}
          </div>
        )}
        {history.map((m, i) => (
          <div key={i} className={`ai-msg ai-msg-${m.role}`}>
            <div className="ai-msg-role">{m.role}</div>
            <div className="ai-msg-text">{m.content}</div>
            {m.role === "assistant" && toolCallsByTurn[i] && (
              <div className="ai-tool-calls">
                {toolCallsByTurn[i].map((tc, j) => (
                  <details key={j} className="ai-tool-call">
                    <summary>
                      <code>{tc.tool_name}</code>{" "}
                      <span className="muted small">
                        {summariseArgs(tc.args)}
                      </span>
                    </summary>
                    <div className="ai-tool-call-body">
                      <div className="muted small">args</div>
                      <pre>{prettyJson(tc.args)}</pre>
                      <div className="muted small">result</div>
                      <pre>{prettyJson(tc.result)}</pre>
                    </div>
                  </details>
                ))}
              </div>
            )}
          </div>
        ))}
        {busy && (
          <div className="ai-msg ai-msg-assistant ai-msg-typing">
            <div className="ai-msg-role">assistant</div>
            <div className="ai-msg-text">
              <span className="ai-typing-dot" />
              <span className="ai-typing-dot" />
              <span className="ai-typing-dot" />
            </div>
          </div>
        )}
        {error && (
          <div className="ai-msg-error">
            <strong>Error:</strong> {error}
          </div>
        )}
      </div>

      {lastUsage && (
        <div className="ai-panel-usage" title="Token usage of the last reply">
          in {lastUsage.in} · out {lastUsage.out} · cached{" "}
          {lastUsage.cache_read}
          {cacheHitPct !== null && ` (${cacheHitPct}%)`}
          {lastUsage.cache_write > 0 &&
            ` · wrote ${lastUsage.cache_write}`}
        </div>
      )}

      <div className="ai-panel-controls">
        <label className="ai-panel-toggle">
          <input
            type="checkbox"
            checked={includeGraph}
            onChange={(e) => setIncludeGraph(e.currentTarget.checked)}
          />
          include graph context
          {selectedEntity && (
            <span className="muted" style={{ marginLeft: 6 }}>
              · selection: {selectedEntity.label}
            </span>
          )}
        </label>
        <label className="ai-panel-toggle" title="Let Claude call transforms, search the graph, and add entities autonomously. Pivots are recorded under the &quot;ai-analyst&quot; actor in the audit log.">
          <input
            type="checkbox"
            checked={allowTools}
            onChange={(e) => setAllowTools(e.currentTarget.checked)}
          />
          allow tools (autonomous pivoting)
        </label>
      </div>

      <form
        className="ai-panel-input"
        onSubmit={(e) => {
          e.preventDefault();
          send();
        }}
      >
        <textarea
          ref={inputRef}
          value={input}
          rows={2}
          placeholder="Ask the analyst… (Enter to send, Shift+Enter for newline)"
          onChange={(e) => setInput(e.currentTarget.value)}
          onKeyDown={onKeyDown}
          disabled={busy}
        />
        <button
          type="submit"
          disabled={busy || !input.trim() || status?.configured === false}
          title={
            status?.configured === false
              ? "Set ANTHROPIC_API_KEY first"
              : "Send (Enter)"
          }
        >
          {busy ? "…" : "Send"}
        </button>
      </form>
    </aside>
  );
}

/// Pretty-print a JSON-string for the tool-call expand-cards. Falls
/// back to the raw text when the input isn't valid JSON.
function prettyJson(s: string): string {
  try {
    return JSON.stringify(JSON.parse(s), null, 2);
  } catch {
    return s;
  }
}

/// One-line summary of tool args for the collapsed `<summary>` row.
/// We surface the most-investigator-meaningful fields: entity_id /
/// transform_id / label. Falls back to a compact JSON line.
function summariseArgs(s: string): string {
  try {
    const v = JSON.parse(s) as Record<string, unknown>;
    if (typeof v.transform_id === "string" && typeof v.entity_id === "string") {
      return `→ ${v.transform_id} on ${(v.entity_id as string).slice(0, 8)}…`;
    }
    if (typeof v.query === "string") return `"${v.query}"`;
    if (typeof v.entity_id === "string") {
      return `${(v.entity_id as string).slice(0, 8)}…`;
    }
    if (typeof v.label === "string") return `"${v.label}"`;
    return JSON.stringify(v).slice(0, 60);
  } catch {
    return "";
  }
}
