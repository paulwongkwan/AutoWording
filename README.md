# AutoWording

**A native Windows desktop application that uses a vision LLM to generate microstock metadata (Title, Caption, 50 keywords) and writes it directly into JPEG EXIF / IPTC / XMP tags.**

| Item | Value |
| --- | --- |
| Language | Rust 2021 (minimum `rust-version = 1.85`; verified with 1.99.1) |
| GUI | `eframe` / `egui` 0.28.1, OpenGL (`glow`), custom Windows 11 light theme, Segoe UI |
| AI transport | `reqwest` 0.12 + `tokio` multi-thread runtime, `native-tls` (Windows Schannel) |
| Metadata engine | ExifTool command-line executable (external dependency) |
| Platform | Windows 10/11 (code paths and build instructions are Windows-specific) |
| Binary | `target\release\autowording.exe` (~21 MB, Windows subsystem, no console window) |
| Tests | **78** unit/integration tests, all passing as of 2026-11-05 |
| State | Single-crate application, no workspace, no CI, no installer, no license file |

This README is written to be sufficient context for another LLM or developer to modify the project safely **without reading every source file first**. It documents not only what the app does, but the exact wire formats, prompt text, filesystem contracts, invariants, and known pitfalls.

---

## 1. What the application does

AutoWording is a batch keywording tool for microstock photographers.

1. The user queues JPEG files or folders (drag-and-drop or file dialogs).
2. A vision-capable language model receives the image plus a strict system prompt.
3. The model returns four fields: `Title`, `Editorial Status`, `Caption`, `Keywords`.
4. The reply is parsed tolerantly (markdown, numbered labels, CRLF, full-width colons), normalized, and validated.
5. The user reviews/edits the fields in a desktop UI.
6. ExifTool writes the result into three metadata families simultaneously: **EXIF**, **IPTC**, and **XMP**.
7. Status badges track each image through `Pending → Processing → Generated → Saved`, with `Error` for failures.

The app is intentionally local-first: files never leave the machine except when a cloud provider (OpenAI or Anthropic) is selected. Ollama and LM Studio run locally. API keys are stored on disk in plain text.

---

## 2. Agent quick contract (read this before changing code)

These are the non-obvious rules that make the app correct. Breaking one is likely to reintroduce a bug that already has a regression test.

1. **JPEG only.** The `image` crate is compiled with only the `jpeg` feature. Accepted extensions are `.jpg`, `.jpeg`, `.jpe`, `.jfif` (case-insensitive). Do not claim PNG/TIFF/HEIC support without adding codecs and updating tests.
2. **ExifTool is required for reads and writes.** There is no native Rust metadata writer. If ExifTool is absent, generation still works but save is disabled and existing metadata cannot be loaded.
3. **Keyword replacement requires repeated `=` assignments and no `+=`.** ExifTool treats `=` like `+=` anywhere in a command that also contains `+=`/`-=`. `write_args` MUST emit only `-IPTC:Keywords=value` and `-XMP-dc:Subject=value`. See §11.4.
4. **All ExifTool arguments go through a UTF-8 argfile.** Passing non-ASCII values (e.g. `Café`) through Windows `argv` mangles them to the ANSI code page. Do not "simplify" `ExifTool::run` back to `Command::arg` for tag values or paths.
5. **Saving overwrites the original file.** `-overwrite_original` is always used. There are no `_original` backups and no undo.
6. **The system prompt and metadata mapping are product requirements, not suggestions.** The exact text is reproduced in §10.3. Tests assert specific substrings and parsed output. Do not reword casually.
7. **A batch slot is shared by generation and saving.** Only one batch runs at a time; `self.batch.is_some()` disables batch actions. Do not start a second background batch without redesigning progress accounting.
8. **Metadata loading never overwrites a non-empty editor.** Existing file metadata populates an item only when title, caption, and keywords are all empty and status is `Pending`.
9. **Validation is advisory.** Title word count, caption length, and keyword count produce warning labels but never block generation or saving.
10. **Config is auto-saved, metadata is not.** Provider/prompt/ExifTool settings persist after a 500 ms debounce. Generated metadata is written only when the user saves.
11. **API keys are plaintext** in `config.toml`. Never commit that file or print its contents.
12. **Tests must keep their exact behavior.** In particular, ExifTool integration tests skip (not fail) when the executable is unavailable. Do not make CI assume ExifTool exists.

---

## 3. Feature summary

- Three-pane Windows 11-style UI: queue (left), preview + editor (center), settings/actions (right).
- Drag-and-drop files or folders; recursive folder scan; duplicate suppression.
- Thumbnails (max 128 px) and a large preview (max 1800 px), decoded off the UI thread.
- Four providers behind one interface: Ollama, LM Studio, OpenAI, Anthropic.
- Per-provider endpoint, model, and API key settings.
- Optional per-session situational context; optional persistent extra system-prompt instructions.
- Tolerant response parser and keyword normalizer (dedupe, punctuation stripping, camel-case splitting is **not** performed; see §10.5).
- Live compliance chips: title word count, caption character count, keyword count, editorial indicator.
- Editable title, caption, keyword chips, commercial/editorial toggle.
- Batch generation for all pending/error images; batch save for all dirty generated images.
- Status badges and a `Modified` badge when generated/saved fields are edited.
- ExifTool auto-detection, manual path override, re-check, and version display.
- Config persistence to `%APPDATA%\AutoWording\config\config.toml`.
- Keyboard shortcuts: `↑`/`↓`, `Ctrl+Enter`, `Ctrl+S`, `Ctrl+O`.
- Real ExifTool integration tests for overwrite semantics, 50 keywords, Unicode values, and Unicode file names.

---

## 4. Requirements

### 4.1 Runtime

- Windows 10 or 11 (developed on Windows 11).
- [ExifTool](https://exiftool.org/) installed and either:
  - on `PATH` as `exiftool`, or
  - selected in the **EXIF writer** panel, or
  - placed at one of the auto-detected locations listed in §11.2.
- A vision-capable model:
  - Ollama with a model such as `llava`, `minicpm-v`, `qwen2-vl`; or
  - LM Studio with a vision model and the local server enabled; or
  - an OpenAI API key; or
  - an Anthropic API key.

### 4.2 Build

- Rust stable, at least 1.85.
- A working linker:
  - **Preferred/normal:** MSVC toolchain + Visual Studio Build Tools with the Windows SDK, or
  - **This machine's setup:** GNU toolchain `stable-x86_64-pc-windows-gnu` plus the portable linker tools in `.tools\gnu-bin` (gitignored).
- No C compiler is needed at build time because `reqwest` uses `native-tls`/Schannel instead of `ring`. Do not switch back to `rustls` unless a C toolchain is available; `ring` previously failed to build here.

---

## 5. Build, run, and test

### 5.1 Canonical Windows helper

```bat
build.bat test
build.bat build --release
build.bat run --release
```

`build.bat` prepends `%~dp0.tools\gnu-bin` to `PATH` and invokes:

```bat
cargo +stable-x86_64-pc-windows-gnu %*
```

It exists because the MSVC toolchain on the development machine has no Windows SDK (`kernel32.lib` missing) and Git's `link.exe` shadows the MSVC linker.

### 5.2 Equivalent Bash/Git-Bash command used for verification

```bash
SELF="$HOME/.rustup/toolchains/stable-x86_64-pc-windows-gnu/lib/rustlib/x86_64-pc-windows-gnu/bin/self-contained"
export PATH="$PWD/.tools/gnu-bin:$SELF:$PATH"
cargo +stable-x86_64-pc-windows-gnu test
cargo +stable-x86_64-pc-windows-gnu build --release
```

The extra `SELF` directory provides the GNU linker's self-contained runtime. `build.bat` works without it in the tested environment, but if linking fails with missing DLLs, add it.

### 5.3 Normal MSVC build

With Visual Studio Build Tools and the Windows SDK installed:

```bat
cargo test
cargo run --release
```

### 5.4 Artifacts and ignored paths

- `target/` — Cargo output, gitignored.
- `.tools/` — portable `gcc.exe`, `ld.exe`, `dlltool.exe`, `as.exe`, gitignored.
- `.freebuff/` — local agent workspace metadata, gitignored.
- `%APPDATA%\AutoWording\config\config.toml` — user settings and API keys, outside the repo.

### 5.5 Build profile

`Cargo.toml` sets `[profile.release] opt-level = 2`. There is no LTO, stripping, icon, manifest, or installer configuration.

---

## 6. End-user workflow

1. Launch `autowording.exe`.
2. Add images with **Add images**, **Add folder**, drag-and-drop, or `Ctrl+O`.
3. Pick a provider in the top bar or the **AI provider** panel.
4. Set endpoint/model. Cloud providers also require an API key.
5. Optionally enter **Situational context** (applies to every request this session, not persisted).
6. Optionally expand **Advanced system prompt** and add persistent extra rules.
7. Select an image and press **Generate** / `Ctrl+Enter`, or **Process all pending**.
8. Review the title, editorial toggle, caption, and keyword chips. Click a chip to remove it; type a keyword and press Enter to add one.
9. Press **Save to file** / `Ctrl+S`, or **Save all generated**.
10. The badge changes to **Saved**; editing afterward shows **Modified**.

The UI never auto-saves metadata. Closing the app with unsaved edits loses them (only settings persist).

---

## 7. UI map and interaction details

### 7.1 Layout

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ Top bar: logo, AutoWording, provider combo, ExifTool status                  │
├──────────────┬───────────────────────────────────────────┬───────────────────┤
│ Queue        │ Preview card                              │ AI provider       │
│ (316 px)     │   filename, status, dimensions, ‹ / ›     │ Prompt            │
│              ├───────────────────────────────────────────┤ Batch actions     │
│ thumbnails   │ Metadata editor                           │ EXIF writer       │
│ status pills │   Title / editorial / caption / keywords  │ config path       │
├──────────────┴───────────────────────────────────────────┴───────────────────┤
│ Status bar: toast or Ready · queued / generated / saved counts               │
└──────────────────────────────────────────────────────────────────────────────┘
```

- Window: 1440×920 initial, minimum 1180×720.
- Left panel width: 316 px, not resizable.
- Right panel width: 346 px, not resizable.
- Center editor height: 47% of available height, clamped to 280–430 px; preview gets the rest.
- Preview image max decode dimension: 1800 px; thumbnail max: 128 px.
- Light theme only. Segoe UI is loaded from `C:\Windows\Fonts\segoeui.ttf`; if missing, egui's default font is used.

### 7.2 Statuses

| Status | Badge | Meaning |
| --- | --- | --- |
| `Pending` | Pending (gray) | Newly queued, no metadata generated |
| `Processing` | Processing (blue) | AI request in flight |
| `Generated` | Generated (violet) | AI result parsed successfully |
| `Saved` | Saved (green) | Metadata written successfully |
| `Error` | Failed (red) | Generation failed; message shown under preview |
| `Generated`/`Saved` + `dirty` | Modified (amber) | User edited fields after generation/save |

`Status::Error` counts as pending for batch retry (`ImageItem::is_pending` returns true for `Pending | Error`).

### 7.3 Keyboard shortcuts

| Shortcut | Action | Notes |
| --- | --- | --- |
| `↑` / `↓` | Previous / next image | Only when no text field has focus |
| `Ctrl+Enter` | Generate for the selected image | Consumed globally |
| `Ctrl+S` | Save the selected image | Consumed globally; requires non-empty title |
| `Ctrl+O` | Add images | Consumed globally |

Shortcuts are handled with `consume_key`, so they do not also reach the focused widget.

### 7.4 Queue behavior

- `Add images` filters to `jpg/jpeg/jpe/jfif`.
- `Add folder` recursively walks with `walkdir`, sorted by file name within each directory.
- Dropped files/folders are handled on every frame via `ctx.input().raw.dropped_files`.
- Paths are converted to absolute paths with `std::path::absolute` when possible.
- Duplicate suppression uses exact `PathBuf` equality, not case-insensitive comparison.
- Right-click a row to **Remove from queue**.
- **Clear** empties the queue, selection, thumbnail cache, preview, and metadata-request set.
- Selecting a row requests a preview and (if ExifTool is ready) existing metadata.

### 7.5 Editing semantics

- Title is a single-line field; caption is a multiline field; keywords are removable chips plus an add field.
- **Normalize** runs the same keyword normalizer used by the AI parser.
- `Clear fields` clears title, caption, and keywords locally.
- Editing a `Generated` or `Saved` item sets `dirty = true`; editing a `Pending` or `Error` item does not (it remains pending).
- Compliance warnings come from `prompt::validate` and are recomputed live.
- The editorial toggle is a UI state plus the `editorial` bool on the image item; it does **not** change the caption automatically after generation. The AI is responsible for producing the Shutterstock caption format.

### 7.6 Batch actions

- **Process selected** — one image; equivalent to `Ctrl+Enter`.
- **Process all pending (N)** — all `Pending` and `Error` images.
- **Save all generated (N)** — all items with non-empty title and `dirty == true`.
- A progress bar reports `Processing done/total` or `Saving done/total`.
- There is **no cancel**. Closing the app while a batch runs aborts the process, potentially mid-write.

---

## 8. Architecture

### 8.1 Runtime/thread model

```
┌────────────────────────────────────────────────────────────┐
│ egui UI thread (App::update, ~60 fps)                      │
│  - paints widgets, handles input, owns queue and textures  │
│  - drains WorkerEvent channel in poll_events()             │
│  - debounces and writes config.toml                        │
└───────────────┬────────────────────────────────────────────┘
                │ tokio::spawn / spawn_blocking
                ▼
┌────────────────────────────────────────────────────────────┐
│ tokio multi-thread runtime (2 worker threads)              │
│  - image decode/resize for thumbnails and preview          │
│  - AI request: prepare_image + reqwest HTTP                │
│  - ExifTool read/write in spawn_blocking                   │
│  - sends WorkerEvent back through unbounded mpsc           │
└────────────────────────────────────────────────────────────┘
```

- The runtime is created once in `App::new` with `worker_threads(2)` and `enable_all()`.
- `tokio::sync::mpsc::unbounded_channel` carries `WorkerEvent`.
- UI texture creation happens only on the UI thread in `poll_events`.
- Background tasks call `ctx.request_repaint()` after sending an event.
- `App::update` requests a repaint every 120 ms while a batch is running or a preview is loading.
- Config changes are timestamped in `cfg_dirty`; `poll_events` saves after `CONFIG_SAVE_DELAY` (500 ms), then requests a repaint.

### 8.2 Generation data flow

```
queue paths
  └─ generate_paths()
       ├─ validate model / API key
       ├─ mark each item Processing
       ├─ create BatchProgress { total, done: 0, saving: false }
       └─ spawn async task
            for each path (sequential):
              spawn_blocking(prepare_image)      // decode, resize, JPEG q85, base64
              ai::generate(AiRequest)           // provider HTTP
              send WorkerEvent::AiDone
            send WorkerEvent::BatchFinished
```

`AiDone` replaces the item's title/caption/keywords/editorial wholesale, sets `Generated` and `dirty = true`.

### 8.3 Save data flow

```
save_items(Vec<(PathBuf, Metadata)>)
  ├─ require no active batch and ExifTool present
  ├─ BatchProgress { saving: true }
  └─ spawn async task
       for each item (sequential):
         spawn_blocking(ExifTool::write)
         send WorkerEvent::SaveDone
       send WorkerEvent::BatchSaved
```

`SaveDone` on success sets `Saved`, clears `dirty`, clears the error message. On failure it leaves the status unchanged and sets a `Save failed: …` message.

### 8.4 Metadata load flow

```
select(index)
  ├─ request_preview
  └─ request_metadata (only if ExifTool ready and not already requested)
       spawn_blocking(ExifTool::read)
       send WorkerEvent::MetaLoaded
```

`MetaLoaded` fills dimensions if missing and populates title/caption/keywords **only** when all three are empty and the item is `Pending`. `meta_requested` prevents repeated reads; it is cleared when ExifTool becomes ready and when an item is removed.

### 8.5 Event reference (`src/state.rs::WorkerEvent`)

| Event | Payload | UI handling |
| --- | --- | --- |
| `Thumbnail` | path, `ColorImage` | create texture, cache in `thumbs` |
| `Preview` | path, `ColorImage` | create texture, replace `preview` |
| `ImageFailed` | path, error | mark decode failure, show message |
| `AiDone` | path, `Result<GeneratedMetadata, String>` | fill fields or set `Error` |
| `BatchFinished` | ok, failed | clear batch, show toast |
| `SaveDone` | path, `Result<(), String>` | set `Saved` or message |
| `BatchSaved` | ok, failed | clear batch, show toast |
| `MetaLoaded` | path, `ExistingMetadata` | populate empty pending item |
| `ExifToolReady` | `Option<ExifTool>`, status | store tool, retry metadata for selection |

### 8.6 Core data structures

```rust
enum Status { Pending, Processing, Generated, Saved, Error }

struct ImageItem {
    path: PathBuf,
    status: Status,
    title: String,
    caption: String,
    keywords: Vec<String>,
    editorial: bool,
    dirty: bool,
    message: Option<String>,
    dims: Option<(u32, u32)>,
}

struct GeneratedMetadata { title, caption, keywords, editorial }

struct Metadata { title, caption, keywords }          // what gets written

struct ExistingMetadata {
    title: Option<String>,
    caption: Option<String>,
    keywords: Vec<String>,
    dims: Option<(u32, u32)>,
}

struct BatchProgress { total: usize, done: usize, saving: bool }
```

---

## 9. Source map

```
Cargo.toml                     crate metadata, dependencies, release profile
Cargo.lock                     locked dependency graph (committed)
build.bat                      GNU-toolchain build wrapper
README.md                      this document

src/main.rs                    Windows subsystem attr, egui window setup
src/app.rs                     all UI, workers, batch orchestration, shortcuts (1887 lines)
src/state.rs                   queue/item/status model, JPEG discovery, decode, WorkerEvent
src/config.rs                  Provider enum, ProviderSettings, Config load/save/defaults
src/theme.rs                   colors, egui style, Segoe UI loading, card frame
src/ui.rs                      shared widgets: buttons, chips, status pills, dots

src/ai/mod.rs                  AiRequest, prepare_image, generate dispatch, endpoint, snippet
src/ai/prompt.rs               exact prompt, parser, keyword normalization, validation
src/ai/ollama.rs               Ollama payload and chat
src/ai/lmstudio.rs             LM Studio payload, endpoint normalization, chat
src/ai/openai.rs               OpenAI-compatible payload and chat
src/ai/anthropic.rs            Anthropic Messages payload and chat
src/ai/tests.rs                hand-written TCP mock HTTP server tests for all providers

src/metadata/mod.rs            ExifTool detection, argfile protocol, read/write, integration tests
src/metadata/tags.rs           tag mapping, exiftool argument builder, JSON parser, tag tests
```

Dependency roles:

| Crate | Role |
| --- | --- |
| `eframe`/`egui` 0.28.1 | Native window, OpenGL rendering, widgets |
| `tokio` | Runtime, channels, spawn_blocking |
| `reqwest` 0.12 (`json`, `native-tls`) | HTTP to providers (Schannel on Windows) |
| `serde` / `serde_json` | Config and provider JSON |
| `toml` | Config persistence |
| `directories` 5 | OS config directory |
| `rfd` 0.14 | Native file/folder pickers |
| `walkdir` 2.5 | Recursive folder scan |
| `base64` 0.22 | Image payload encoding |
| `regex` 1.10 | Tolerant field parser |
| `chrono` 0.4 (`clock`) | Local date for the editorial timestamp |
| `anyhow` 1 | Error context |
| `image` 0.25 (`jpeg` only) | Decode, resize, re-encode JPEGs |

---

## 10. AI subsystem

### 10.1 Request contract (`src/ai/mod.rs`)

```rust
struct AiRequest {
    provider: Provider,
    settings: ProviderSettings, // base_url, model, api_key
    image_base64: String,       // no data: prefix
    context: Option<String>,    // trimmed; omitted when empty
    global_prompt: String,      // extra system instructions
    date: String,               // e.g. "November 5, 2026"
}
```

`ai::generate` validates in this order:

1. Model must be non-empty, else `No model configured for <provider>.`
2. Cloud providers require a non-empty API key.
3. Image base64 must be non-empty.
4. Build a `reqwest::Client` with a **300-second timeout**.
5. Dispatch to the provider module.
6. Parse the returned text with `prompt::parse_response`, adding provider context on failure.

There are no retries, backoff, streaming, or cancellation tokens.

### 10.2 Image preparation (`prepare_image`)

- Opens the file with `image::open` (JPEG only in this build).
- If `max(width, height) > 1600`, scales so the longest side is 1600 px, using `FilterType::Triangle`.
- Always converts to RGB8 and re-encodes as JPEG at quality **85**, even if no resize was needed.
- Returns standard Base64 without a `data:` prefix.
- This strips all metadata before the image is sent to a provider.
- EXIF orientation is **not** applied; a portrait photo with an orientation tag may be sent sideways. (Known limitation.)

### 10.3 Exact system prompt

The following text lives in `SYSTEM_TEMPLATE` in `src/ai/prompt.rs`. `{DATE}` is replaced with the batch's local date formatted as `Month D, YYYY` (English month name, no leading zero). Then, if `global_prompt` is non-empty, this is appended:

```
\n\nAdditional instructions from the photographer (follow these too):\n<trimmed global_prompt>
```

Full template:

```text
Role & Task:
You are an expert photography analyst specializing in metadata tag generation, keywording, and stock image description. Analyze the provided image and generate accurate titles, captions, and keywords following strict guidelines.

Analysis Guidelines:
1. Identification Focus:
   - Identify specific landmarks, famous buildings, cities, or locations whenever possible.
   - Specify exact biological classification for fauna and flora (e.g., "Monarch Butterfly - Danaus plexippus", "Quercus robur") rather than broad categories.
   - Detail vehicle makes, models, and types, along with specific technical or industrial objects.
2. Commercial vs. Editorial Status:
   - Check for recognizable human faces.
   - If a recognizable face is present, mark the image as EDITORIAL and construct the caption following the Shutterstock standard format:
     CITY, STATE/COUNTRY – MONTH DAY, YEAR: [Factual sentence describing the subject matter].
   - Use today's date for the editorial timestamp.

Today's date: {DATE}

Output Formatting Constraints:

1. Title:
   - Length: Exactly 3 to 5 words.
   - Content: Clear, descriptive summary of the primary subject.

2. Caption:
   - Length: Strict limit of 50 to 180 characters (including spaces).
   - Content: Factual description of the scene (or Shutterstock editorial format if applicable).

3. Keywords:
   - Quantity: Exactly 50 keywords.
   - Format: Single words only (no multi-word phrases/keyphrases).
   - Separator: Comma-separated.
   - Rules: Alphabetic characters only. No numbers, punctuation, or special characters.

Required Output Schema:
Title: [3-5 words]
Editorial Status: [Commercial / Editorial]
Caption: [50-180 characters]
Keywords: [word1, word2, word3, ... word50]

Reply with the four labelled lines above and nothing else. Do not use markdown.
```

Date formatting is done once per batch in `App::generate_paths`:

```rust
let now = chrono::Local::now();
let date = format!("{} {}, {}", now.format("%B"), now.day(), now.year());
```

### 10.4 User prompt (`user_prompt`)

Without context:

```text
Analyze the attached image and return the four labelled fields exactly as specified.
```

With context (trimmed; empty/whitespace is treated as absent):

```text
Situational context supplied by the photographer (treat it as ground truth):
<context>

Analyze the attached image and return the four labelled fields exactly as specified.
```

### 10.5 Provider wire formats

All providers receive the same system and user prompts and the same base64 JPEG. Differences:

| Provider | Default base URL | Final endpoint | Auth | Image field | Temperature / tokens | Response extraction |
| --- | --- | --- | --- | --- | --- | --- |
| Ollama | `http://localhost:11434` | `/api/chat` (base ending `/api` becomes `/api/chat`) | none | `messages[1].images = [base64]` | `stream: false`, `options.temperature: 0.2` | `message.content`, fallback `response`; checks top-level `error` |
| LM Studio | `http://localhost:1234/v1` | if base ends `/chat/completions` use as-is; if ends `/v1` append `/chat/completions`; otherwise append `/v1/chat/completions` | none | OpenAI-style `image_url` data URI | `temperature: 0.2`, `max_tokens: 900` | `choices[0].message.content` (string) |
| OpenAI | `https://api.openai.com/v1` | `/chat/completions` | `Authorization: Bearer <key>` | `image_url` data URI | `temperature: 0.2`, `max_tokens: 900` | `choices[0].message.content`; string or array of text parts joined with `\n` |
| Anthropic | `https://api.anthropic.com/v1` | `/messages` | `x-api-key`, `anthropic-version: 2023-06-01` | `content[0] = image` with `source.type = base64`, `media_type = image/jpeg` | `temperature: 0.2`, `max_tokens: 900`; system is top-level | `content[]` text blocks joined with `\n` |

`ai::endpoint(base, path)` behavior:

- Trim whitespace and trailing `/`.
- Empty base → return the path (not a usable URL; prevents accidental localhost fallback).
- If base already ends with the exact path → return base unchanged.
- Special case: `/api/chat` with base ending `/api` → `base/chat`.
- Otherwise `base + path`.

HTTP/network failures include a human hint for Ollama/LM Studio ("Is the server running?"). Non-2xx bodies are truncated with `snippet` (400 chars, whitespace collapsed, `…` suffix). JSON parse errors include the snippet. Missing expected fields produce provider-specific errors.

### 10.6 Tolerant reply parser

Field labels are found with a case-insensitive regex that accepts:

- Start of line, whitespace, comma, or semicolon before the label.
- Optional numbering: `1.`, `2)`, etc.
- Optional markdown decorations: `*`, `#`, `>`, `-`.
- Labels: `Title`, `Editorial Status`, `Editorial`, `Caption`, `Description`, `Keywords`, `Keyword`.
- Optional emphasis asterisks after the label.
- ASCII `:` or full-width `：`.

A field's value runs from the colon to the start of the next recognized label (or end of text), so captions and keyword lists may span lines. Values are cleaned by:

- Trimming whitespace, `*`, backticks, and underscores at the edges.
- Collapsing internal whitespace runs to one space.
- Removing wrapping double quotes.
- Title additionally loses a trailing period.

Duplicate labels: the **last** occurrence wins (the map is inserted in order).

Missing title, caption, or keywords produces an error listing the missing fields plus a 400-character raw reply snippet. Editorial defaults to Commercial; the word `commercial` anywhere in the status wins over `editorial` (so "Not editorial - commercial use only" → false). Otherwise the presence of `editorial` sets `editorial = true`.

### 10.7 Keyword normalization

`normalize_keywords(raw)` is shared by AI parsing and the UI **Normalize** button:

1. Split on comma, semicolon, newline, CR, `|`, or `/`.
2. Split each chunk on whitespace (so multi-word phrases become separate words).
3. Keep only alphabetic characters (`char::is_alphabetic`; Unicode letters survive, including `Café` and `北京`).
4. Drop results shorter than 2 characters.
5. Deduplicate case-insensitively, preserving the first spelling and order.
6. Stop at exactly 50.

Consequences:

- `2024` → dropped (no letters ⇒ empty).
- `3d` → `d` → dropped (1 char).
- `blue-sky` → `bluesky`.
- `New York` → `New`, `York`.
- `a` / `I` → dropped.
- Non-ASCII punctuation is removed; non-Latin scripts are preserved.
- Case variants (`Sunset`, `sunset`) count as one, keeping the first.

### 10.8 Validation (advisory only)

`validate(title, caption, keywords)` returns strings for:

- Title word count not in `3..=5` (split on whitespace).
- Caption character count not in `50..=180` (`chars().count()`, so Unicode scalar values, not bytes).
- Keyword count not exactly 50.
- First keyword that is not at least 2 alphabetic characters or contains a non-alphabetic character.

The UI renders these as amber bullets. Saving is still allowed.

---

## 11. Metadata subsystem

### 11.1 Tag mapping (product requirement)

| Logical field | Tags written |
| --- | --- |
| Title | `XMP-dc:Title`, `IPTC:ObjectName`, `EXIF:XPTitle` |
| Caption | `EXIF:ImageDescription`, `IPTC:Caption-Abstract`, `XMP-dc:Description` |
| Keywords | repeating `IPTC:Keywords`, repeating `XMP-dc:Subject` |

If the caption is empty but the title is non-empty, **only** `EXIF:ImageDescription` is set to the title. If the title is empty, no title tags are written. If both are empty, no descriptive tags are written.

If the keyword list is empty, **no keyword arguments are emitted**, so existing keywords in the file are left untouched. The UI cannot clear a keyword list. (To implement clearing, add an explicit empty assignment once and verify ExifTool semantics; do not mix `=` and `+=`.)

### 11.2 ExifTool detection order

`ExifTool::detect(preferred)` tries, in order:

1. `preferred` from config:
   - if it is a directory → `<dir>\exiftool.exe`, `<dir>\exiftool`
   - else use the path as-is
2. Next to the executable: `exiftool.exe`
3. `<exe>\resources\exiftool\exiftool.exe`
4. `<exe>\resources\exiftool.exe`
5. `<cwd>\resources\exiftool\exiftool.exe`
6. `<cwd>\resources\exiftool.exe`
7. `exiftool` from `PATH`

Each candidate is accepted only if `exiftool -ver` exits successfully and prints a non-empty version. Detection runs in `spawn_blocking`. The version is displayed in the UI and stored in `ExifTool::version`.

### 11.3 ExifTool invocation protocol

Every read/write goes through `ExifTool::run`:

1. Build a `Vec<String>` of arguments.
2. Write them to `%TEMP%\autowording-exif-<pid>-<counter>.args` as UTF-8, one argument per line.
3. Flatten `\r`/`\n` inside each argument to spaces.
4. Execute:

```text
<exiftool> -charset filename=utf8 -@ <argfile>
```

5. Remove the argfile after the process completes (success or failure).
6. On Windows, set `CREATE_NO_WINDOW` (0x0800_0000) so no console flashes.

Why an argfile:

- Windows converts `argv` to the process's ANSI code page, so `Café` became `Cafe` in earlier versions.
- A UTF-8 argfile preserves accented and CJK characters in both values and file paths.
- `-charset filename=utf8` tells ExifTool that file names in the argfile are UTF-8, so Unicode file names work.

Do **not** reintroduce direct `Command::arg` for tag values or paths. Direct arguments are still used for `-ver` (ASCII only) and for the fixed `-charset filename=utf8 -@ <argfile>` invocation.

### 11.4 Write arguments

Base flags, always present and in this order:

```text
-P -overwrite_original -charset iptc=utf8 -m
```

- `-P` preserves the file's modification date/time.
- `-overwrite_original` overwrites in place and suppresses `_original` backups.
- `-charset iptc=utf8` writes IPTC as UTF-8.
- `-m` ignores minor metadata errors.

Then title tags (if non-empty), caption tags (if non-empty, else optional `ImageDescription` fallback), then keywords.

**Critical keyword rule:** every keyword is emitted as its own `=` assignment:

```text
-IPTC:Keywords=bridge
-IPTC:Keywords=sunrise
-XMP-dc:Subject=bridge
-XMP-dc:Subject=sunrise
```

ExifTool documentation states that using `=` is equivalent to `+=` in any command where the same tag is also set with `+=` or `-=`. Therefore, mixing `=` and `+=` preserves the old list. Repeated pure `=` assignments replace the list with exactly the supplied values. The regression test `overwriting_metadata_replaces_previous_keywords` verifies this against real ExifTool.

### 11.5 Read arguments and precedence

Arguments:

```text
-j -charset iptc=utf8
-Title -ObjectName -Description -Caption-Abstract -ImageDescription
-XPTitle -IPTC:Keywords -Subject -ImageSize
<path>
```

JSON parsing (`parse_read_json`):

- Requires a JSON array with at least one object.
- Title preference: `Title` → `XPTitle` → `ObjectName`; empty/whitespace values fall through.
- Caption preference: `Description` → `Caption-Abstract` → `ImageDescription`.
- Keywords: merge `Keywords` then `Subject`; each may be an array of strings or a comma-separated string; trim, drop empties, dedupe case-insensitively (first spelling/order preserved).
- `ImageSize` is parsed only as lowercase `WIDTHxHEIGHT`; uppercase `X` is rejected.
- Invalid JSON, an empty array, or an object instead of an array is an error.
- Values of the wrong JSON type are ignored rather than failing the whole read.
- Output is decoded with `String::from_utf8_lossy` before JSON parsing.

The UI's `READ_TAGS` uses short names (`-Title`, `-Description`, `-Subject`) because ExifTool resolves them to the preferred group. The parser still checks the specific fallback keys in case the output includes them.

### 11.6 File and error behavior

- `write` rejects a path that is not an existing file before invoking ExifTool.
- `read` does not pre-check; ExifTool's failure is surfaced.
- A non-zero exit is an error. Write errors use stderr if non-empty, otherwise stdout, trimmed.
- Success is determined solely by exit status; ExifTool's "1 image files updated" text is not parsed.
- Each file is a separate ExifTool process. There is no directory batching.
- If the argfile cannot be written, the write/read fails with that context.
- If the process cannot be spawned (e.g. tool deleted after detection), the error says `Could not run exiftool at <path>`.

---

## 12. Configuration

### 12.1 Location

`Config::config_path()` uses:

```rust
directories::ProjectDirs::from("", "", "AutoWording")
    .config_dir()
    .join("config.toml")
```

On Windows, `ProjectDirs::config_dir()` appends `config`, so the real path on the development machine is:

```text
C:\Users\<user>\AppData\Roaming\AutoWording\config\config.toml
```

(The settings panel displays the exact resolved path.) If `ProjectDirs` fails, it falls back to the executable's directory, then to the relative `config.toml`.

`Config::load()`:

- Reads the file if present.
- On a TOML parse error, prints a message to stderr and uses **all defaults** (including default API keys).
- On a missing file, uses defaults and immediately saves them.
- Sets `cfg.path` to the resolved path. `path` is `#[serde(skip)]`, so it is never written.

`Config::save()` creates parent directories and writes pretty TOML. The UI debounces saves by 500 ms after any change.

### 12.2 Schema and defaults

```rust
enum Provider { Ollama, LmStudio, OpenAi, Anthropic } // serde: snake_case
struct ProviderSettings { base_url: String, model: String, api_key: String }
struct Config {
    provider: Provider,
    ollama: ProviderSettings,
    lmstudio: ProviderSettings,
    openai: ProviderSettings,
    anthropic: ProviderSettings,
    global_prompt: String,
    exiftool_path: String,
    #[serde(skip)] path: Option<PathBuf>,
}
```

| Field | Default |
| --- | --- |
| `provider` | `ollama` |
| `ollama.base_url` | `http://localhost:11434` |
| `ollama.model` | `llava` |
| `lmstudio.base_url` | `http://localhost:1234/v1` |
| `lmstudio.model` | `local-model` |
| `openai.base_url` | `https://api.openai.com/v1` |
| `openai.model` | `gpt-4o-mini` |
| `anthropic.base_url` | `https://api.anthropic.com/v1` |
| `anthropic.model` | `claude-sonnet-4-20250514` |
| all `api_key` fields | empty |
| `global_prompt` | empty |
| `exiftool_path` | empty (auto-detect) |

TOML provider values are exactly `ollama`, `lm_studio`, `open_ai`, `anthropic`. An unknown value is a parse error and causes the whole config to fall back to defaults. `#[serde(default)]` on `Config` and `ProviderSettings` means missing fields/sections are filled from defaults.

Example:

```toml
provider = "open_ai"
global_prompt = "Prefer British spelling and avoid brand names."
exiftool_path = ""

[ollama]
base_url = "http://localhost:11434"
model = "llava"
api_key = ""

[lmstudio]
base_url = "http://localhost:1234/v1"
model = "local-model"
api_key = ""

[openai]
base_url = "https://api.openai.com/v1"
model = "gpt-4o-mini"
api_key = "sk-..."

[anthropic]
base_url = "https://api.anthropic.com/v1"
model = "claude-sonnet-4-20250514"
api_key = ""
```

### 12.3 Security

- API keys are stored **in plain text**. The UI masks the field but the file is not encrypted.
- `config.toml` lives outside the repository. Never commit it, print it, or paste its contents into logs.
- No telemetry, analytics, auto-update, or crash reporting exists.
- Network traffic goes only to the selected provider endpoint. Ollama and LM Studio traffic stays local when their base URLs are local.
- Images are re-encoded and downscaled before transmission, which removes metadata as a side effect.
- ExifTool runs locally with the user's permissions.

---

## 13. Tests

### 13.1 Commands

```bat
build.bat test
```

or:

```bash
SELF="$HOME/.rustup/toolchains/stable-x86_64-pc-windows-gnu/lib/rustlib/x86_64-pc-windows-gnu/bin/self-contained"
export PATH="$PWD/.tools/gnu-bin:$SELF:$PATH"
cargo +stable-x86_64-pc-windows-gnu test
```

Expected: `78 passed; 0 failed; 0 ignored`.

List tests:

```bash
cargo test -- --list
```

### 13.2 Test inventory

| Module | Count | Coverage |
| --- | --- | --- |
| `ai::prompt::tests` | 24 | markdown/CRLF/numbered/full-width labels, duplicate labels, multiline values, editorial precedence, title cleanup, keyword normalization/Unicode/dedupe/cap, validation boundaries, exact caption preservation |
| `ai::tests` | 17 | mock HTTP server for all four providers; payload/header/endpoint assertions; malformed JSON; HTTP errors; missing fields; image resize/encoding/rejection; snippet truncation |
| `config::tests` | 6 | defaults, serde names (`lm_studio`, `open_ai`), accessors, save/load round-trip, invalid TOML, path stability |
| `metadata::tags::tests` | 13 | all written tags, title fallback, empty metadata, keyword replacement (no `+=`), Unicode arguments, read precedence/fallbacks/merge/dedupe/invalid payloads |
| `metadata::tests` | 8 | real ExifTool round-trip, overwrite semantics, 50 keywords, Unicode values, Unicode file names, missing files, explicit path/directory detection, argfile formatting |
| `state::tests` | 10 | extension recognition, recursive collection, single/missing paths, decode resize/no-resize/corrupt, item conversion/status, progress fraction, selection stepping/removal |

ExifTool integration tests call `ExifTool::detect("")`; if no tool is found they print `exiftool not available; skipping test` and return successfully. This is intentional so the suite can run on machines without ExifTool, but it means those eight tests are not evidence of metadata correctness unless ExifTool is installed. On the development machine ExifTool is installed and all eight execute.

Provider tests do **not** contact real APIs. `src/ai/tests.rs` contains a small single-request TCP HTTP server bound to `127.0.0.1:0`; it reads the request, asserts method/path/headers/body, and returns canned JSON. No API keys or network access are required.

Test artifacts are written under `%TEMP%\autowording-tests`, `%TEMP%\autowording-state-tests`, and `%TEMP%\autowording-config-tests`.

### 13.3 What is deliberately not tested

- egui rendering, widget interaction, drag-and-drop, and keyboard shortcuts (no UI automation).
- Live Ollama/LM Studio/OpenAI/Anthropic calls.
- Windows file dialogs (`rfd`).
- ExifTool behavior on malformed/corrupt JPEGs.
- Concurrent batch saves, cancellation, or process interruption.
- Long-running memory behavior with thousands of images.
- Non-Windows builds.

---

## 14. Invariants, gotchas, and design rationale

1. **Repeated `=` replaces; `+=` appends.** Mixing them makes `=` behave like `+=`. The code and test `keyword_arguments_use_replacement_not_append` enforce no `+=`.
2. **UTF-8 argfile is mandatory on Windows.** Direct `argv` loses accents/CJK. `unicode_metadata_round_trips` and `unicode_file_names_round_trip` protect this.
3. **`-overwrite_original` is destructive.** No backup files are created. The README and UI should not imply undo.
4. **Metadata is only loaded into empty pending items.** This prevents a late-arriving `MetaLoaded` from clobbering a generation result or user edit.
5. **Generation replaces all four fields**, including a manually edited caption. There is no merge/confirm prompt.
6. **Save requires a non-empty title.** Caption-only or keyword-only items cannot be saved from the UI, even though the metadata layer could write them.
7. **Validation never blocks save.** A one-word title or 3 keywords will still be written.
8. **Only one batch runs at a time**, and generation/saving share the slot. A stuck provider request blocks other batches for up to 300 s.
9. **Sequential processing.** Images in a batch are handled one at a time; two tokio workers do not parallelize the per-image loop.
10. **`batch.done` counting is mode-dependent.** `AiDone` increments only when `!saving`; `SaveDone` increments only when `saving`.
11. **Selection is clamped by pure helpers.** `stepped_index(None, n, ...)` returns `Some(0)`; an out-of-range selection is clamped. `index_after_removal` keeps a valid neighbor or returns `None` for an empty queue.
12. **`Config::load` falls back wholesale on a parse error**, not field-by-field. A single invalid provider value resets all settings in memory (the bad file remains on disk until the next save).
13. **`path` is skipped by serde.** Tests assert it does not round-trip through TOML.
14. **The config path includes a `config` directory** on Windows because of `ProjectDirs::config_dir()`. Do not document it as `%APPDATA%\AutoWording\config.toml`; that was wrong.
15. **Anthropic uses `x-api-key`, not Bearer.** OpenAI uses Bearer. LM Studio and Ollama send no auth header.
16. **LM Studio endpoint normalization is special.** It accepts a bare host, a `/v1` base, or a full `/chat/completions` URL.
17. **Ollama accepts a base ending in `/api`.** `endpoint` turns it into `/api/chat`.
18. **No automatic retry.** Transient 429/500/network errors surface immediately as `Error`; the user can reprocess.
19. **The AI sees a re-encoded JPEG, not the original.** Quality 85 and max 1600 px may hide fine detail (e.g. small species markings).
20. **No EXIF orientation correction.** The image crate decodes raw pixels; a rotated photo may be analyzed sideways.
21. **Keywords are single words by design.** Multi-word concepts are split, which can reduce stock relevance; the prompt asks the model to emit single words anyway.
22. **Keyword dedupe is case-insensitive but preserves the first spelling.** `Bridge` then `bridge` yields `Bridge`.
23. **Unicode keywords are allowed by validation** (`is_alphabetic`), even though the prompt asks for "alphabetic characters only".
24. **The editorial bool does not alter the caption.** If the model ignores the Shutterstock format, toggling Editorial will not fix it.
25. **The app is not code-signed.** SmartScreen may warn. There is no installer or auto-update.

---

## 15. How to extend safely

### 15.1 Add a provider

1. Add a variant to `Provider` in `src/config.rs`.
2. Update `Provider::ALL`, `label`, `is_cloud`, `requires_key`, `default_base_url`, `default_model`, `base_hint`, `model_hint`, `guidance`.
3. Add a `ProviderSettings` field to `Config`, its default, and the `settings`/`settings_mut` match arms.
4. Create `src/ai/<provider>.rs` with `payload(...)` and `chat(...)`, following the existing patterns.
5. Declare the module in `src/ai/mod.rs` and add a match arm in `generate`.
6. Add mock-server tests to `src/ai/tests.rs` (payload shape, endpoint, auth header, response parsing, HTTP error).
7. Add the provider to config round-trip tests.
8. Update this README's provider table and defaults.

Exhaustive `match` arms will force most of these updates at compile time.

### 15.2 Change the prompt or output schema

1. Edit `SYSTEM_TEMPLATE` / `system_prompt` / `user_prompt` in `src/ai/prompt.rs`.
2. If fields change, update `GeneratedMetadata`, `parse_fields`, `parse_response`, and the missing-field error.
3. Update `validate` and the UI warning chips in `src/app.rs`.
4. Update `metadata/tags.rs` and `state.rs` if the new field must be written.
5. Add parser tests for old and new formats; keep tolerance for markdown.
6. Update §10.3 of this README verbatim.

### 15.3 Add or change a metadata tag

1. Update `Metadata` and/or `ExistingMetadata` in `src/metadata/tags.rs`.
2. Update `write_args` (and avoid mixing `=` with `+=`).
3. Update `READ_TAGS` and `parse_read_json` with an explicit precedence order.
4. Update `ImageItem`, `to_metadata`, and the UI editor.
5. Update tests for argument generation and real round-trip behavior.
6. Update the mapping table in §11.1.

### 15.4 Change keyword semantics

Edit `normalize_keywords` only, because the AI parser and UI Normalize button share it. Preserve:

- Unicode letters.
- Case-insensitive dedupe with first spelling.
- The 50-keyword cap.
- Splitting on `, ; newline CR | /` and whitespace.

Then update the normalization tests and §10.7.

### 15.5 Add UI automation

The app has no test hooks or headless mode. To add UI tests you would need to refactor `App` to separate pure state transitions from egui rendering (e.g. an `AppCore` with an event-in/event-out API), or drive the real window with a Windows UI automation tool. Do not claim UI coverage without such work.

---

## 16. Troubleshooting

| Symptom | Likely cause / fix |
| --- | --- |
| `ExifTool not found — install it or set its path` | Install ExifTool, add it to `PATH`, or browse to `exiftool.exe` in the EXIF writer panel and press **Re-check**. |
| `Could not run exiftool at …` | The configured path disappeared or is not executable; press **Use PATH** or re-select. |
| Save button disabled | No ExifTool, no active selection, or a batch is running. |
| `Generate or type a title before saving` | The UI requires a non-empty title. |
| Keywords were not replaced | Check `write_args` for `+=`; see §11.4. |
| Accented text became plain ASCII | Arguments are going through `argv`; ensure `ExifTool::run` still uses the UTF-8 argfile. |
| OpenAI/Anthropic 401 | Missing/incorrect API key; keys are per-provider. |
| Ollama/LM Studio connection refused | Start the local server; check the base URL and port. |
| LM Studio 404 on `/v1/v1/...` | The endpoint normalizer should prevent this; verify `lmstudio::endpoint_for`. |
| Model returns unparseable text | The error includes a 400-char snippet. Use a stronger instruction-following vision model; the parser tolerates markdown but needs the four labels. |
| Metadata appears in one program but not another | Different readers prioritize different tag families. The app writes EXIF, IPTC, and XMP; verify with `exiftool -a -G1 -s file.jpg`. |
| Build fails with `kernel32.lib` / `link.exe` errors | Use `build.bat` (GNU toolchain) or install the Windows SDK and ensure the correct `link.exe` is on `PATH`. |
| Build fails in `ring` | `reqwest` was switched away from `rustls`; do not re-enable it without a C compiler. |
| Unicode file name is not found by ExifTool | Ensure `-charset filename=utf8` is still passed and the path is inside the argfile. |
| Tests pass but metadata tests were skipped | ExifTool was not detected; install it and rerun. Check stderr for `exiftool not available; skipping test`. |

Useful diagnostic command:

```bat
exiftool -a -G1 -s "path\to\image.jpg"
```

---

## 17. Limitations and non-goals

- Windows-only target; no macOS/Linux packaging or testing.
- JPEG only; no PNG, TIFF, HEIC, RAW, or video.
- No XMP sidecar workflow; writes into the JPEG itself.
- No batch cancellation, pause, resume, or per-image retry button (use **Process all pending**).
- No request retries/backoff, rate-limit handling, or token/cost accounting.
- No automatic EXIF orientation correction before sending to the model.
- No metadata diff, preview of raw ExifTool output, or undo.
- No keyword translation, synonym expansion, or per-market keyword limits.
- No support for clearing existing keywords when the new list is empty.
- No project/session persistence beyond config; the queue is in memory only.
- No installer, code signing, auto-update, tray icon, or single-instance enforcement.
- No CI configuration. Tests are run manually.
- No license file. Add one before redistributing; the absence of a license means all rights are reserved by default.

---

## 18. Glossary

| Term | Meaning |
| --- | --- |
| **Microstock** | Low-cost royalty-free/editorial stock photography marketplaces (Shutterstock, Adobe Stock, etc.). |
| **Editorial** | Content that documents newsworthy/recognizable people or events; usually cannot be used commercially without releases. |
| **Commercial** | Content usable for advertising; recognizable faces/property generally require releases. |
| **IPTC** | Legacy news metadata standard; `Keywords`, `ObjectName`, `Caption-Abstract`. |
| **XMP** | Adobe's XML metadata framework; `dc:Title`, `dc:Description`, `dc:subject`. |
| **EXIF** | Camera/image metadata; `ImageDescription`, `XPTitle`. |
| **ExifTool** | Phil Harvey's Perl-based command-line metadata reader/writer used by this app. |
| **Argfile** | A UTF-8 text file passed with `-@`, one ExifTool argument per line, used to bypass Windows code-page conversion. |
| **Shutterstock caption format** | `CITY, STATE/COUNTRY – MONTH DAY, YEAR: Factual sentence.` |
| **Dirty** | UI flag indicating generated/saved fields were edited and need saving. |

---

## 19. Verification record

Last verified on the development machine (Windows 11, 2026-11-05):

- `cargo +stable-x86_64-pc-windows-gnu test` → **78 passed, 0 failed**.
- `cargo +stable-x86_64-pc-windows-gnu build --release` → success, `target/release/autowording.exe` ~21 MB.
- Real ExifTool tests executed (ExifTool detected on `PATH`).
- README claims about Unicode argfiles, keyword replacement, and config path were verified against the running code/tests.
- The GUI was launched in an earlier smoke run but was **not** visually inspected in this verification pass; no screenshots are committed.

When updating this section, report exactly what ran. Do not claim live provider calls or UI automation unless they were actually performed.
