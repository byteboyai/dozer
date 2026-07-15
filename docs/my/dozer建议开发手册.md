Byteboy Dozer

The AI Workspace for Builders

Version: MVP Vision Draft

⸻

1. Vision

Dozer is not another AI IDE.

It is a native AI Workspace designed for the agent era.

Traditional IDEs place the editor at the center.

Dozer places the workspace and AI agents at the center.

Developers should be free to use Cursor, Zed, VS Code or any other editor, while Dozer becomes the command center that orchestrates AI agents, terminals, browsers, documents and project resources.

⸻

2. Product Philosophy

Traditional IDE

Project
↓
Folder
↓
File
↓
Editor
↓
AI Assistant

AI is only a feature of the editor.

⸻

Dozer

Workspace
↓
Agents
↓
Tasks
↓
Resources

Resources include:

* Files
* Browser Pages
* Documents
* PDFs
* Images
* Logs
* Terminal Sessions

Editors become external tools.

⸻

3. Positioning

Dozer is:

* AI Workspace
* Agent Runtime Manager
* Workspace Orchestrator

Dozer is NOT:

* IDE
* Code Editor
* AI Chat Application
* Terminal Emulator

⸻

4. Core Principles

Agent First

AI Agents are first-class citizens.

Every agent has:

* Session
* Runtime
* Status
* PTY
* Timeline
* Context

⸻

Workspace First

Everything belongs to a Workspace.

Workspace contains:

* Agents
* Resources
* Tasks
* Timeline
* Panes
* Plugins

⸻

Native First

Entire application built with Rust.

No Electron.

No Web IDE.

⸻

Plugin First

Everything should be extensible.

Examples:

* Git
* Docker
* Browser
* MCP
* Timeline
* AI Memory

⸻

5. MVP Scope

Workspace

* Create Workspace
* Open Workspace
* Persistent Layout
* Workspace Snapshot
* Restore Workspace

⸻

Agent Manager

Support launching:

* Claude Code
* Codex CLI
* Gemini CLI
* Qwen
* boy (future)

Features:

* Start
* Stop
* Restart
* Status
* Runtime Information

⸻

Terminal Pane

Multiple PTYs.

Support:

* Split
* Tabs
* Persistent Sessions
* Shell
* Agent Terminal

⸻

Project Navigator

A lightweight project navigator.

Not an IDE Explorer.

Features:

* Tree View
* Recent Files
* AI Recently Modified
* Reveal in Finder
* Open with External Editor

⸻

Preview Pane

Read-only viewer.

Support:

* Markdown
* PDF
* Images
* JSON
* YAML
* TOML
* CSV
* Office Documents

Rendering engine:

* Flyfish File Viewer

⸻

Browser Pane

Embedded browser.

Use cases:

* Documentation
* GitHub
* AI Research
* Localhost Preview

Future:

* Browser Use
* Playwright
* MCP Browser

⸻

Command Palette

Global command launcher.

Examples:

* Open Workspace
* Launch Agent
* Switch Pane
* Search Resource

⸻

Timeline

Record:

* Agent Actions
* Commands
* File Changes
* Builds
* Tests

⸻

6. Pane System

Everything displayed inside Dozer is a Pane.

Pane
├── Terminal
├── Preview
├── Browser
├── Timeline (future)
├── Diff (future)
├── Logs (future)
├── Docker (future)
├── MCP (future)

Every pane shares a common lifecycle.

Pane
Open
Close
Save State
Restore State

⸻

7. Workspace Structure

Workspace
│
├── Agents
│
├── Tasks
│
├── Resources
│     ├── Files
│     ├── PDFs
│     ├── Images
│     ├── Browser Pages
│     ├── Logs
│     └── Artifacts
│
├── Timeline
│
├── Panes
│
└── Plugins

⸻

8. Resources

Resources are more important than files.

Examples:

* Source Code
* PDF
* Markdown
* Images
* Browser Tabs
* API Documentation
* Logs

All resources should be searchable.

⸻

9. Things We Will NOT Build

Dozer intentionally avoids becoming another IDE.

Not included in MVP:

* Code Editor
* IntelliSense
* Debugger
* Refactoring
* Language Server
* Theme Marketplace
* SSH Manager
* AI Chat UI

Editing is delegated to:

* Cursor
* Zed
* VS Code

⸻

10. Event Architecture

Everything communicates through events.

Examples:

AgentStarted
AgentStopped
PromptSent
TerminalOutput
FileChanged
BrowserOpened
TaskFinished

This enables:

* Plugins
* Timeline
* Notifications
* Automation

⸻

11. Runtime Architecture

UI
│
├── Workspace
├── Sidebar
├── Dock
└── Pane
│
▼
Application Layer
│
▼
Domain
│
├── Workspace
├── Agent
├── Task
├── Resource
└── Timeline
│
▼
Runtime
│
├── PTY
├── Terminal
├── Browser
├── Preview Engine
└── Plugins
│
▼
Infrastructure
│
├── SQLite
├── Config
├── File System
├── Git
└── Logging

⸻

12. Suggested Technology Stack

GUI

* iced

⸻

Terminal

* alacritty_terminal
* portable-pty

⸻

Async

* tokio

⸻

Serialization

* serde

⸻

Database

* SQLite

⸻

Logging

* tracing

⸻

Configuration

* TOML

⸻

File Watching

* notify

⸻

Git

* gitoxide

⸻

Preview

* Flyfish File Viewer

⸻

Browser

Platform-native WebView

⸻

13. Future Roadmap

Phase 1

MVP

* Workspace
* Terminal
* Browser
* Preview
* Agent Manager

⸻

Phase 2

Workspace Intelligence

* Timeline
* Task Manager
* Prompt History
* Search
* Workspace Memory

⸻

Phase 3

Agent Platform

* Browser Agent
* MCP
* Docker Runtime
* Loop Engine
* Multi-Agent Collaboration

⸻

Phase 4

AI Workspace Ecosystem

* Plugin Marketplace
* Cloud Workspace
* Team Collaboration
* Remote Runtime

⸻

Final Philosophy

Traditional IDEs are built around editing code.

Dozer is built around building software with AI.

Editors edit files.

Dozer orchestrates work.

# 14. Technology Selection

The goal of Dozer is to build a **native, high-performance, extensible AI Workspace** with a long-term maintainable architecture.

The following components are recommended for the MVP.

| Module | Recommended | Reason |
|----------|------------|--------|
| GUI Framework | **iced** | Native Rust GUI, modern architecture, reactive design, cross-platform |
| Dock Layout | **iced_dock** (or custom implementation) | Multi-pane workspace similar to modern IDEs |
| Terminal Emulator | **alacritty_terminal** | Battle-tested terminal core with excellent performance |
| PTY | **portable-pty** | Cross-platform PTY abstraction |
| Async Runtime | **tokio** | Rust async ecosystem standard |
| Event Bus | `tokio::broadcast` + `mpsc` | Decoupled event-driven architecture |
| File Watching | **notify** | Cross-platform filesystem monitoring |
| Configuration | **serde + toml** | Simple and idiomatic Rust configuration |
| Serialization | **serde** | Rust ecosystem standard |
| Database | **SQLite (rusqlite or sqlx)** | Lightweight workspace persistence |
| Logging | **tracing** | Structured logging and diagnostics |
| Error Handling | **thiserror + anyhow** | Clean error model |
| Search | **nucleo** | Extremely fast fuzzy searching |
| Git | **gitoxide** | Pure Rust Git implementation |
| File System | **camino** | UTF-8 safe paths |
| Directory Utilities | **directories** | Cross-platform user directories |
| UUID | **uuid** | Workspace, pane and agent identifiers |
| Time | **chrono** | Timeline and workspace history |
| Process Management | **tokio::process** | Async process execution |
| Plugin Interface | Trait-based abstraction (future: WASI Component Model) | Long-term plugin architecture |

---

# 15. UI Components

## Workspace

- iced
- iced_dock

Purpose:

- Multi-pane workspace
- Drag & Drop tabs
- Split layout
- Persistent layouts

---

## Terminal Pane

Components:

- alacritty_terminal
- portable-pty

Features:

- Multiple PTYs
- Split terminal
- Agent terminal
- Shell terminal
- Persistent sessions

---

## Preview Pane

Rendering Engine:

- Flyfish File Viewer

Supported Formats:

- Markdown
- PDF
- Images
- Office Documents
- JSON
- YAML
- TOML
- CSV

Future:

- AI Summary
- Annotation
- Diff
- Semantic Search

---

## Browser Pane

Platform-native WebView

macOS:

- WKWebView

Windows:

- WebView2

Linux:

- WebKitGTK

Purpose:

- Documentation
- GitHub
- Localhost Preview
- AI Research

Future:

- Browser Use
- Playwright
- MCP Browser

---

## Project Navigator

Components:

- notify
- camino

Features:

- File Tree
- Recent Files
- AI Recently Modified
- Drag & Drop
- Open in External Editor

---

## Command Palette

Search Engine:

- nucleo

Inspired by:

- VS Code
- Raycast

Features:

- Workspace Commands
- Resource Search
- Agent Search
- File Search

---

# 16. Future Integrations

These components are intentionally **not included in the MVP**, but should be considered in the long-term architecture.

## AI Runtime

- Claude Code
- Codex CLI
- Gemini CLI
- OpenHands
- Qwen
- Byteboy boy

---

## Browser Automation

- Browser Use
- Playwright
- Chromium CDP

---

## AI Memory

- SQLite
- LanceDB
- Qdrant
- sqlite-vec

---

## Knowledge Search

- Tantivy
- Meilisearch

---

## Data Processing

- Apache Arrow
- Polars

---

## Workflow Engine

- Temporal
- Dagu
- Windmill
- Trigger.dev

---

## Docker

- bollard

---

## Git Hosting

- GitHub API
- gitoxide

---

## MCP

- Rust MCP SDK
- MCP Client
- MCP Server

---

## Document Parsing

- Flyfish File Viewer
- PDFium
- pulldown-cmark

---

## Observability

- tracing
- tracing-subscriber
- OpenTelemetry

---

# 17. Design Guidelines

When introducing new dependencies, follow these principles.

## Prefer Rust-native libraries

Avoid wrapping Node.js or Electron-based solutions whenever a mature Rust alternative exists.

---

## Minimize external runtime dependencies

Dozer should not require:

- Node.js
- Python
- Java

unless absolutely necessary.

---

## Platform-native integration

Prefer native APIs over embedded runtimes.

Examples:

- WKWebView
- WebView2
- WebKitGTK

instead of Chromium when possible.

---

## Extensibility

Every major subsystem should be replaceable.

For example:

Preview Pane

```text
Preview Pane
        │
        ▼
Preview Engine
        │
        ├── Flyfish
        ├── PDFium
        ├── Markdown
        └── Future Engines
```

Similarly:

```text
Terminal Pane
        │
        ▼
Terminal Engine
        │
        ├── alacritty_terminal
        ├── wezterm_term (future)
        └── Future Engines
```

This abstraction ensures that Dozer remains maintainable and adaptable as the ecosystem evolves.