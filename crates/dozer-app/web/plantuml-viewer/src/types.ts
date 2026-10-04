//! Protocol shapes shared with Rust (`preview/webview_protocol.rs`). Field names
//! are snake_case on the wire; this module mirrors the spec §5.2 command/event
//! set. Keep in sync with the Rust side — a mismatch is a protocol bug.
//!
//! Wire shape: the shared `WebviewEnvelope<T>` carries binding/revision at the
//! top level and the command/event as `payload`; the payload itself is a
//! `#[serde(tag = "kind")]` tagged enum, so every variant has a `kind` field.

export const PROTOCOL_VERSION = 1;

/** `WebviewEnvelope<T>` from the Rust side. */
export interface Envelope<T> {
  protocol_version: number;
  project_id: number;
  panel: string;
  tab_id: number;
  document_id: string;
  revision: number;
  request_id: string | null;
  payload: T;
}

/** One project-relative include file, content already authorized/read by Rust. */
export interface PlantUmlInclude {
  /** Project-relative normalized path, e.g. `docs/diagrams/common.puml`. */
  path: string;
  /** UTF-8 file contents. */
  content: string;
}

export type PlantUmlTheme = "light" | "dark";

export type PlantUmlCommand =
  | {
      kind: "set_document";
      revision: number;
      path: string;
      source: string;
      includes: PlantUmlInclude[];
      theme: PlantUmlTheme;
    }
  | { kind: "fit_view" }
  | { kind: "actual_size" }
  | { kind: "reset_view" };

export type PlantUmlFailureKind =
  | "syntax"
  | "too_large"
  | "engine"
  | "timeout"
  | "remote_include"
  | "internal";

/**
 * Host → Rust events. `revision` lives on the envelope, not the payload, so
 * `rendered`/`failed` do not repeat it (spec §5.2).
 */
export type PlantUmlEvent =
  | { kind: "ready" }
  | {
      kind: "rendered";
      width: number;
      height: number;
      duration_ms: number;
    }
  | {
      kind: "failed";
      failure_kind: PlantUmlFailureKind;
      message: string;
      line: number | null;
    }
  | { kind: "open_source"; line: number | null };
