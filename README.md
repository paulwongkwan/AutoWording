# AutoWording

**A native Windows 11 ACDSee-style desktop application that uses a vision LLM to generate microstock metadata (Title, Caption, adjustable title-word and caption-character ranges, and an adjustable keyword target defaulting to 50) for one image or a multi-selection, then writes it directly into JPEG EXIF / IPTC / XMP tags.**

| Item | Value |
| --- | --- |
| Language | Rust 2021, minimum `rust-version = 1.85` (verified with 1.99.1) |
| GUI | `eframe` / `egui` 0.28.1, OpenGL (`glow`), Windows 11 light/dark appearance, Segoe UI |
| Workspace | ACDSee Classic dual-mode: **Browser** (This PC folder tree + thumbnail grid + preview) and **Viewer** (single image + filmstrip) |
| Selection | Path-based multi-selection with Ctrl+Click, Shift+Click, Ctrl+Shift+Click, Ctrl+A, and checkbox overlays |
| AI transport | `reqwest` 0.12 + `tokio` multi-thread runtime, `native-tls` (Windows Schannel) |
| Batch engine | Sequential AI processing (one image at a time), live `Queued`/`Processing`/`Generated`/`Saved`/`Error` badges |
| Metadata engine | ExifTool command-line executable (external dependency) |
| Platform | Windows 10/11 (build and runtime are Windows-specific) |
| Binary | `target\release\autowording.exe` (~21 MB, Windows subsystem, no console window) |
| Tests | **127** unit/integration tests, all passing as of 2026-11-06 |
| State | Single crate, no workspace, no CI, no installer, no license file |

This README is the operating manual for another LLM or developer. It records the exact prompt, wire formats, tag mappings, selection rules, batch semantics, filesystem contracts, and pitfalls that are easy to regress.

---

## 1. Agent quick contract (read before changing code)

1. **JPEG only.** The `image` crate is compiled with only the `jpeg` feature. Accepted extensions are `.jpg`, `.jpeg`, `.jpe`, `.jfif` (case-insensitive). Do not claim PNG/TIFF/HEIC/RAW support without adding codecs and tests.
2. **ExifTool is required for metadata reads and writes.** There is no native Rust metadata writer. Without ExifTool, AI generation works but Save is disabled and existing metadata is not loaded.
3. **Keyword replacement requires repeated `=` assignments and no `+=`.** ExifTool treats `=` like `+=` in any command that also contains `+=`/`-=`. `writer::write_args` MUST emit only `-IPTC:Keywords=value` and `-XMP-dc:Subject=value`.
4. **Every ExifTool argument goes through a UTF-8 argfile.** Windows `argv` code-page conversion mangles `Café` to `Cafe`. Do not "simplify" `ExifTool::run` back to direct tag/path arguments.
5. **Saving overwrites the original JPEG.** `-overwrite_original` is always used. The configurable `.autowording-backups` copy preserves the first pre-write version, but there is still no undo.
6. **The prompt and tag mapping are product requirements.** The exact system prompt is in §11.3; tests assert substrings and parsed outputs. Do not reword casually.
7. **The advisory validation does not block saving.** Title-word and caption-character range checks and the configured keyword target only produce warning labels.
8. **Selection is path-based.** `Selection` stores `HashSet<PathBuf>` plus `active` and `anchor`. Do not replace it with grid indices; statuses must survive folder reloads and reordering.
9. **The grid shows the current folder's direct JPEGs**, plus any explicitly added files, sorted case-insensitively. Folder scanning is non-recursive; adding a dropped folder is recursive.
10. **Metadata loading never clobbers generated/edited content.** `MetaLoaded` fills only an empty item whose status is `Pending`.
11. **Only one batch runs at a time.** Generation and saving share `App::batch`; all batch buttons are disabled while it is `Some`.
12. **AI batches are sequential and pausable.** One image is prepared and generated at a time; Pause stops before the next image and Cancel stops after the current request. Statuses go `Queued → Processing` in order.
13. **Saving requires a non-empty title.** Selected items without a title are skipped and counted in a toast.
14. **AI generation auto-saves metadata.** After an AI batch finishes, every successful result is written through ExifTool; the app still supports explicit **Save** after manual edits. Provider/prompt/ExifTool/folder/theme/density/output-range/keyword-target/backup/verification settings persist after a 500 ms debounce.
15. **API keys are plaintext** in `config.toml`. Never commit, print, or log that file.

---

## 2. What the application does

AutoWording is a batch keywording workspace for microstock photographers. The workflow follows ACDSee Photo Studio's Browser/Viewer model:

1. Browse This PC in the left system folder tree, then open a stock folder. The center grid shows that folder's direct JPEGs.
2. Select one image (single inspector) or many (batch inspector).
3. Run AI on the selection. Images are processed one at a time: each request sends a downscaled JPEG plus the strict prompt and the image's existing metadata (title, caption, keywords, GPS location, capture date) and session context. The active marketplace profile's title range, caption range, and keyword target are embedded in the prompt.
4. Successful AI results are written to the JPEG automatically through ExifTool (title, caption, keywords into EXIF/IPTC/XMP). Review/edit Title, Editorial status, Caption, and keyword chips. In batch mode, inspect common/mixed values and apply title/caption/keyword changes across the selection.
5. After manual edits, use **Save** to write the updated metadata. Successful generation results are already on disk.
6. Thumbnails show `Pending`, `In queue`, `Processing`, `Generated`, `Saved`, `Failed`, or `Modified` badges.
7. Switch to Viewer for a large single-image view with previous/next navigation and a filmstrip.

Cloud providers receive only the re-encoded, downscaled image. Ollama and LM Studio keep traffic local. Files are modified in place.

---

## 3. Feature summary

- ACDSee-style menu bar: **File / Edit / View / Theme / Tools / Help**, plus **Browser | Viewer** mode toggle.
- Three-pane Browser: lazy Windows-style system folder browser (276 px) with Quick access, mounted drive roots, clickable breadcrumb navigation, and a compact refresh control; responsive thumbnail grid with multi-select, active preview; 372 px metadata inspector.
- Selection: plain click, Ctrl+Click toggle, Shift+Click contiguous range, Ctrl+Shift+Click additive range, Ctrl+A, and per-card checkbox overlays.
- Visual states: selected accent border, active focus ring, status badges, checkbox checkmarks, hover feedback.
- Appearance: **Follow Windows**, **Light**, or **Dark** from the Theme menu. The choice persists in config; System mode re-checks the Windows app theme when the window regains focus.
- Density: **Comfortable / Compact** in the top bar. Compact uses 126×146 cards, a taller grid area, and tighter metadata spacing and field heights. The choice persists in config.
- Browser selection summary: selected count, total keyword tags, coverage against the configured keyword target, and the number/percentage of selected images with keywords. Coverage waits for selected metadata reads, showing loading, ExifTool-waiting, or unavailable states, and flags unreadable images.
- Single inspector: editable title/caption/editorial/keywords, live compliance chips, add/remove/normalize keywords, and AI field locks.
- Batch inspector: selection count, common/mixed fields (`<Multiple values>`), keyword cloud with occurrence counts, batch title/caption overwrite, keyword append/replace, and batch AI field locks.
- Sequential batch AI: one image at a time, per-image live status, pause/resume, cancel-after-current, partial-failure reporting, retry-failed action.
- AI context: requests include the loaded title, caption, keywords, GPS location, capture date, and session context; generated title, caption, and keywords replace the existing editable values unless the field is locked.
- Output limits in AI Settings: keyword target **1–50**, title **1–20 words**, caption **10–1000 characters**. Title and caption are min–max ranges. A reply outside either range or short on keywords triggers one corrective request. A valid retry replaces an out-of-range title/caption; keywords are merged so the first result is kept and only new unique terms are appended. The selected marketplace profile caps these values further (Adobe Stock: 49 keywords; Alamy: 150 caption characters).
- AI retries: each image gets one request plus up to **three retries** (300/600/900 ms apart) when the provider request fails or the reply cannot be parsed. A reply missing Title/Caption is sent back to the model with a repair instruction and the previous text; only after all attempts fail does the item become `Error` with the last parse error and raw snippet.
- Marketplace profiles: **General stock**, **Adobe Stock**, **Shutterstock**, **iStock**, **Dreamstime**, **Alamy**, **Depositphotos**, and **Pond5**, each with its own keyword, title-word, and caption-character caps and defaults. Switching a profile adopts that agency's defaults and clamps further edits; the chosen profile persists in config.
- Safe writes: optional pre-write copies in `.autowording-backups`, plus optional read-back verification of title, caption, and keywords after every write.
- Batch save: selected items with titles written sequentially, progress bar, per-file errors.
- Viewer mode: large active image, previous/next, filmstrip, error overlay, Escape returns to Browser.
- Folder browser is a lazy/cached **This PC** tree of mounted drives and their folders, plus Quick access shortcuts; it auto-expands to the current folder and supports clickable breadcrumbs and `Alt+Up` parent navigation.
- Four providers: Ollama, LM Studio, OpenAI, Anthropic.
- Endpoint model discovery: fetch/refresh a model dropdown from Ollama `/api/tags`, LM Studio and OpenAI `/v1/models`, and Anthropic `/v1/models` with provider authentication.
- ExifTool auto-detection, manual path override, version display, re-check, Unicode-safe argfile execution.
- Config persistence: `%APPDATA%\AutoWording\config\config.toml`, including appearance, density, marketplace profile, title/caption ranges, keyword-target, backup, and verification preferences.
- Keyboard shortcuts: `Ctrl+O`, `Ctrl+Shift+O`, `Ctrl+A`, `Ctrl+S`, `Ctrl+Enter`, `F5`, `Alt+Up`, arrows, `Esc`.
- 127 tests: prompt/parser, range-aware prompt and validation, target-aware normalization, singular/plural de-duplication, title/caption, keyword, and malformed-reply retry, field-lock merging, safe-write backup/verification, all providers and model-list endpoints via mock HTTP, metadata tags including GPS/date, real ExifTool round-trips, selection math, system folder tree and breadcrumbs, cache lifecycle, config, Windows theme detection, palette application, density preferences, and selection coverage math.

---

## 4. Requirements

### 4.1 Runtime

- Windows 10 or 11.
- [ExifTool](https://exiftool.org/) installed and:
  - on `PATH` as `exiftool`, or
  - selected in **AI Settings → EXIF writer**, or
  - placed in one of the auto-detected locations in §12.2.
- A vision model:
  - Ollama with `llava`, `minicpm-v`, `qwen2-vl`, etc.; or
  - LM Studio with a vision model and local server enabled; or
  - OpenAI API key; or
  - Anthropic API key.

### 4.2 Build

- Rust stable ≥ 1.85.
- A working linker:
  - MSVC toolchain + Visual Studio Build Tools + Windows SDK, or
  - GNU toolchain `stable-x86_64-pc-windows-gnu` plus portable linker tools in `.tools\gnu-bin` (this machine's setup, gitignored).

`reqwest` uses `native-tls`/Schannel, so no C compiler is required. Do not switch back to `rustls` without a C toolchain; `ring` previously failed here.

---

## 5. Build, run, test

### 5.1 Canonical helper

```bat
build.bat test
build.bat build --release
build.bat run --release
```

`build.bat` prepends `%~dp0.tools\gnu-bin` to `PATH` and calls:

```bat
cargo +stable-x86_64-pc-windows-gnu %*
```

It exists because the MSVC toolchain on this machine lacks the Windows SDK (`kernel32.lib` missing) and Git's `link.exe` shadows the MSVC linker.

### 5.2 Verified Bash command

```bash
SELF="$HOME/.rustup/toolchains/stable-x86_64-pc-windows-gnu/lib/rustlib/x86_64-pc-windows-gnu/bin/self-contained"
export PATH="$PWD/.tools/gnu-bin:$SELF:$PATH"
cargo +stable-x86_64-pc-windows-gnu test
cargo +stable-x86_64-pc-windows-gnu build --release
```

The extra `SELF` directory supplies the GNU linker's self-contained runtime.

### 5.3 Normal MSVC build

With Visual Studio Build Tools and the Windows SDK:

```bat
cargo test
cargo run --release
```

### 5.4 Ignored paths

- `target/` — Cargo output.
- `.tools/` — `gcc.exe`, `ld.exe`, `dlltool.exe`, `as.exe`.
- `.freebuff/` — local agent metadata.
- `%APPDATA%\AutoWording\config\config.toml` — user settings and API keys.

`[profile.release] opt-level = 2`; no LTO, stripping, icon, manifest, installer, code signing, or auto-update.

---

## 6. End-user workflow

### 6.1 Open and browse

1. Launch `autowording.exe`. The left panel shows **This PC**, mounted drives, and Quick access folders. The last folder from config is reopened and revealed automatically if it still exists.
2. Expand a drive or folder, or use a Quick access shortcut. Clicking a folder loads its direct JPEGs into the grid.
3. **File → Open Folder…** (`Ctrl+O`) jumps directly to any folder.
4. Click a breadcrumb above the tree or press `Alt+Up` to browse a parent directory.
5. **Add images…** (`Ctrl+Shift+O`) appends arbitrary JPEGs; dropped folders are scanned recursively.
6. **F5** rescans drives, refreshes the current folder, and clears cached tree children while preserving expansion.

### 6.2 Select

- **Click** a card: select only that image, set active focus, show it in preview.
- **Ctrl+Click**: toggle that card without clearing others.
- **Shift+Click**: select the contiguous range from the anchor to the target.
- **Ctrl+Shift+Click**: add that range to the existing selection.
- **Ctrl+A** (when no text field is focused): select all grid images.
- **Checkbox** (top-left of a card): toggle that image without clearing others.
- **Clear** in the grid header empties the selection.
- **↑/↓** moves the active image and selects it singly (when not typing).
- Double-clicking a card switches to Viewer.

The **active** image is the primary focus, shown in the center preview/Viewer and used for the single inspector. It can be part of a larger selection.

### 6.3 Single-image editing

With exactly one selected image, the right panel shows:

- File name, status badge, dimensions.
- Title field + configured word-range chip.
- Commercial/Editorial toggle.
- Caption field + configured character-range chip.
- Keyword chips (click `×` to remove), count chip, **Normalize**.
- Keyword add field (Enter or **Add**).
- Advisory validation bullets.
- AI field locks: Title / Caption / Keywords. Locked fields survive AI results.
- **Generate** (`Ctrl+Enter`, saves the successful result automatically) and **Save to file** (`Ctrl+S`) for manual edits.
- **Clear fields** clears the three metadata fields locally.

Editing a `Generated` or `Saved` image sets `dirty` and changes the badge to **Modified**. Editing a `Pending`/`Error` image leaves the status but makes it saveable once a title exists.

### 6.4 Batch selection editing

With 2+ images selected, the inspector shows:

- `Multi-Selection (N items)` banner.
- Aggregated title/caption: the shared value, or `<Multiple values>` in amber.
- Aggregated editorial status: `Commercial`, `Editorial`, or `<Multiple values>`.
- Keyword cloud: union of keywords with per-keyword counts, sorted by count then alphabetically (max 80 displayed).
- **Batch edit**:
  - Title field + **Apply title to all** (overwrites every selected title).
  - Caption field + **Apply caption to all**.
  - Keywords field + **Append keywords** (dedupe, cap 50) or **Replace keywords**.
- **Batch Run AI (N)**: processes all selected images, including saved ones; successful results are saved automatically when the batch finishes.
- AI field locks in the batch inspector apply the same lock set to every selected image.
- While an AI batch runs, the status bar exposes **Pause/Resume** and **Cancel** (cancel waits for the current image).
- **Save Selected to EXIF (N)**: writes all selected images with non-empty titles.
- **Retry failed (K)**: reprocesses selected `Error` items.
- Status summary and batch progress bar.

Batch fields are not a live merged editor; they are explicit apply operations. A blank title/caption cannot be applied. Keywords use the same normalizer as the AI parser (alphabetic words, dedupe, max 50).

### 6.5 Save and auto-save

- AI generation auto-saves every successful result after the AI batch completes. The write is sequential through ExifTool and uses `-overwrite_original`.
- With **Back up originals before writing** enabled (default), each JPEG is copied to `.autowording-backups` before its first write. Repeated saves keep the first backup.
- With **Verify saved metadata** enabled (default), the file is read back after writing and the save fails if title, caption, or keywords do not match.
- If ExifTool is unavailable, generation still completes, but the result stays `Generated` and the app reports that it was not saved.
- Single: **Save to file** writes the active image.
- Batch: **Save Selected to EXIF** writes every selected image with a non-empty title.
- Items without a title are skipped; the toast reports the skipped count.
- Success sets `Saved` and clears `dirty`; failure keeps the previous status and shows `Save failed: …`.
- Manual edits made after generation are not auto-saved; use Save before closing.

### 6.6 Viewer

- Switch with **View → Viewer**, the top-right mode toggle, or a thumbnail double-click.
- Large image with dark letterboxing, previous/next buttons, position (`3 / 24`), dimensions, status.
- Error message overlay at the bottom of the image.
- Horizontal filmstrip of 58 px thumbnails with status dots; click to change active image.
- `↑/↓` navigates; `Esc` returns to Browser.

### 6.7 Choose a provider and model

1. Expand **AI Settings** in the right inspector.
2. Choose the provider. Its endpoint is prefilled with the provider default and remains editable.
3. The model list is fetched automatically from the configured endpoint when the settings panel is opened, when the provider changes, and after an endpoint or API-key edit loses focus. **Fetch** re-queries on demand.
4. Choose a model from the dropdown. The list contains endpoint-provided IDs. If no model is configured, the first fetched model is selected; the retired LM Studio placeholder `local-model` is never inserted. A configured real model missing from the current response is still shown so it can be retained.
5. If discovery fails, the error is shown inline and the model remains an editable text field; a manually entered model ID is still usable.
6. Cloud providers require an API key before their model list can be fetched. Local providers need no key.

The list is sorted case-insensitively, deduplicated, and limited to model IDs (`name` for Ollama, `id` for the OpenAI-compatible and Anthropic APIs). Model discovery does not call the chat/generation endpoint.

### 6.8 Marketplace profiles

**AI Settings → Marketplace profile** selects the stock agency the output is written for. Choosing a profile adopts that agency's default output limits; the keyword slider and the title/caption range editors are then capped by the profile, so the prompt, the validation chips, and the keyword-coverage summary all follow the same limits.

| Profile | Keywords (default / max) | Title words (default / max) | Caption characters (default / max) |
| --- | --- | --- | --- |
| General stock | 50 / 50 | 3–5 / 20 | 50–180 / 1000 |
| Adobe Stock | 49 / 49 | 3–7 / 20 | 50–200 / 200 |
| Shutterstock | 50 / 50 | 5–9 / 20 | 50–200 / 1000 |
| iStock | 50 / 50 | 3–7 / 20 | 50–200 / 255 |
| Dreamstime | 50 / 50 | 3–7 / 20 | 50–200 / 250 |
| Alamy | 50 / 50 | 3–7 / 12 | 50–150 / 150 |
| Depositphotos | 50 / 50 | 3–7 / 12 | 50–200 / 250 |
| Pond5 | 50 / 50 | 3–7 / 10 | 50–350 / 500 |

Caps mirror each agency's published upload limits (keyword counts and title/description character limits) converted to the app's units; titles are measured in words at roughly 12 characters per word. The app's global bounds still apply, and the metadata writer never emits more than 50 keywords. Shutterstock requires at least five title words, so that profile's default title range starts at 5. Switching profiles never rewrites existing metadata: it changes the defaults and caps for the next generation and for the validation chips. The choice is stored as `marketplace` in `config.toml`.

---

## 7. UI reference

### 7.1 Layout

```
┌────────────────────────────────────────────────────────────────────────────────┐
│ File  Edit  View  Theme  Tools  Help                   [Browser | Viewer]      │
├──────────────────┬─────────────────────────────────────────┬───────────────────┤
│ FOLDER BROWSER   │  FOLDER HEADER  [n] [selected]  Refresh │ METADATA          │
│  Refresh                                                 │  single / batch   │
│  C:\ › Photos › 2026_Stock                                │  Title            │
│  Quick access    │  ┌──────┬──────┬──────┬──────┐         │  Editorial        │
│   Desktop        │  │ card │ card │ card │ card │         │  Caption          │
│   Pictures       │  └──────┴──────┴──────┴──────┘         │  Keywords         │
│  This PC         │  ACTIVE PREVIEW                         │  AI Generator     │
│   ▾ C:\          │  ┌───────────────────────────────┐      │  [Batch Run AI]   │
│     ▾ Photos     │  │      large primary image      │      │  AI Settings ▸    │
│       ▸ 2026_Stock│ └───────────────────────────────┘      │                   │
│   ▸ D:\          │                                        │                   │
│                  │                                        │                   │
├──────────────────┴─────────────────────────────────────────┴───────────────────┤
│ Status: 24 images loaded | 3 selected          Ollama (Connected: qwen2-vl)     │
└────────────────────────────────────────────────────────────────────────────────┘
```

- Window: 1440×920 initial, minimum 1180×720.
- Left panel: 276 px, fixed.
- Right panel: 372 px, fixed.
- Grid max height: 55% of center height, clamped 180–560 px (Compact: 60%, clamped 150–560 px).
- Grid card: Comfortable 156×180 px, 7 px padding; Compact 126×146 px, 5 px padding. Dimensions appear in a dark overlay on the thumbnail.
- Compact metadata inspector: 9×6 px frame margin, 4.5 px line spacing, 25 px field height, reduced section gaps, and a 2-line caption field.
- Browser header summary (when a selection exists): `N selected · N keyword tags · N% of target · N/N images keyworded (N%)`. Tag coverage compares total keywords with the configured target per selected image; image coverage counts selected images with at least one keyword. The summary waits for selected metadata reads before showing percentages, displays loading/ExifTool-unavailable states, and flags unreadable images.
- Thumbnails: max 160 px decode. Preview: max 2200 px decode.
- Viewer filmstrip: 58 px squares, 82 px strip height.
- Light and dark palettes cover panels, cards, menus, inputs, status badges, preview letterboxing, and custom-painted cards. Segoe UI from `C:\Windows\Fonts\segoeui.ttf`; default egui font if absent.

### 7.2 Menus

| Menu | Items |
| --- | --- |
| File | Open Folder… (`Ctrl+O`), Add Images… (`Ctrl+Shift+O`), Save Selected to EXIF (`Ctrl+S`), Exit |
| Edit | Select All (`Ctrl+A`), Clear Selection, Run AI on Selected (`Ctrl+Enter`), Save Selected |
| View | Browser, Viewer, Refresh Folder (`F5`) |
| Theme | Follow Windows, Light, Dark |
| Tools | Re-check ExifTool, About / Diagnostics |
| Help | About AutoWording |

The top-right Browser/Viewer toggle and the adjacent **Comfortable / Compact** switch apply without losing selection.

### 7.3 Statuses

| Status | Badge | Meaning |
| --- | --- | --- |
| `Pending` | Pending (gray) | No metadata generated |
| `Queued` | In queue (blue) | Waiting for the current image to finish |
| `Processing` | Processing (blue) | AI request in flight |
| `Generated` | Generated (violet) | Parsed AI result |
| `Saved` | Saved (green) | Written to disk |
| `Error` | Failed (red) | Generation/save error; message on item |
| `Generated`/`Saved` + dirty | Modified (amber) | Edited after generation/save |

`ImageItem::is_pending()` is true for `Pending | Error`; `Queued` is not retried by that helper. `retry_failed` explicitly filters `Error`.

### 7.4 Keyboard shortcuts

| Shortcut | Action | Notes |
| --- | --- | --- |
| `Ctrl+O` | Open folder | Global |
| `Ctrl+Shift+O` | Add images | Global |
| `Ctrl+A` | Select all grid images | Only when no text field has focus |
| `Ctrl+Enter` | Run AI on selection | Global |
| `Ctrl+S` | Save selection | Global |
| `F5` | Refresh folder/tree | Global |
| `Alt+Up` | Open parent folder | Only when no text field has focus |
| `↑` / `↓` | Previous/next active image | Only when no text field has focus |
| `Esc` | Leave Viewer | Only in Viewer |

Shortcuts use `consume_key`, so they do not also reach text fields.

### 7.5 Grid card details

- Rounded white card; selected cards use a light blue fill and accent border.
- The active card gets a thicker accent focus ring.
- Thumbnail area is dark-neutral; failed images show `Unreadable`, loading shows `Loading…`.
- File name truncated to 19 characters (15 in Compact); dimensions at the bottom-right.
- Status badge painted at the bottom-left.
- Checkbox is visible on every card; checked cards use the accent fill; 17 px hit area (15 in Compact).
- Tooltip shows full path, status, and keyword count.
- Hover does not load metadata; selection does.

### 7.6 System folder browser

- **Quick access** lists Desktop, Downloads, Documents, and Pictures when those system folders exist.
- **This PC** lists mounted drive roots (`C:\`, `D:\`, …); drives are lazy and can be expanded in place.
- Directory children load only when a folder is expanded and are cached per directory.
- Directories sort case-insensitively; `$RECYCLE.BIN` and `System Volume Information` are omitted.
- Opening a folder reveals and expands its whole ancestor chain inside the system tree instead of replacing the tree root.
- Current folder is highlighted and shows its direct-image count.
- Opening a folder clears selection, preview, and metadata-request cache.
- **Refresh** / `F5` is available in the sidebar header and rescans mounted drives, clears tree caches, and preserves expansion state.
- Click any breadcrumb above the tree to jump to an ancestor folder; `Alt+Up` also moves to the parent.
- The grid always shows only the current folder's direct JPEGs, not images from its subfolders.

---

## 8. Architecture

### 8.1 Runtime/thread model

```
┌────────────────────────────────────────────────────────────┐
│ egui UI thread (App::update)                               │
│  - paints Browser/Viewer/metadata panels                   │
│  - owns items, folder_images, Selection, textures          │
│  - drains WorkerEvent channel in poll_events()             │
│  - debounces and writes config.toml                        │
└───────────────┬────────────────────────────────────────────┘
                │ tokio::spawn / spawn_blocking
                ▼
┌────────────────────────────────────────────────────────────┐
│ tokio multi-thread runtime (2 worker threads)              │
│  - thumbnail/preview decode + resize                       │
│  - AI: prepare_image + reqwest HTTP                        │
│  - ExifTool read/write in spawn_blocking                   │
│  - sends WorkerEvent through unbounded mpsc                │
└────────────────────────────────────────────────────────────┘
```

- Runtime created once with `worker_threads(2)` and `enable_all()`. AI generation itself is serialized; the runtime still allows thumbnail/preview and ExifTool work alongside the active request.
- UI texture creation happens only on the UI thread.
- Background tasks call `ctx.request_repaint()` after events.
- `App::update` requests repaint every 120 ms while a batch runs or a preview loads.
- Config changes set `cfg_dirty`; `poll_events` saves after 500 ms.

### 8.2 Folder-open flow

```
open_root(path)
  ├─ folder_of(path): directory or parent of a file
  ├─ folder_root = dir; FolderTree::set_root
  ├─ cfg.last_folder = dir (debounced save)
  └─ open_folder(dir)
       ├─ clear selection/preview/metadata-request cache
       ├─ collect_folder_jpegs(dir) (non-recursive, sorted)
       ├─ ensure ImageItem for each file
       ├─ thumbs.retain(current paths)
       ├─ selection.retain_existing(...)
       └─ request preview + metadata for selected images
```

Changing folders clears the image selection. Refreshing the same folder keeps selections whose files still exist, but an empty selection stays empty; the app does not auto-select the first image. `add_paths` instead uses `collect_jpegs` (recursive) and appends new paths to the grid; if no folder is open, the first file's parent becomes the root.

### 8.3 AI batch flow

```
generate_paths(paths)
  ├─ reject if batch active / empty / no model / cloud key missing
  ├─ dedupe paths, snapshot existing title/caption/keywords/GPS/date, set items to Queued
  ├─ BatchProgress { total, done: 0, saving: false }
  └─ spawn async coordinator
       └─ for each path, sequentially:
            ├─ check Cancel; wait while Pause is active
            ├─ send AiStarted → UI sets Processing
            ├─ spawn_blocking(prepare_image)
            ├─ ai::generate(AiRequest { existing, context, ... })
            ├─ apply generated fields except locked ones
            ├─ count ok/failed and send AiDone
            └─ start the next image only after this one finishes
       └─ after the loop: return untouched Queued items to Pending
          and send BatchFinished
```

`AiDone` replaces the existing Title/Caption/Keywords with the generated values plus Editorial unless that field is locked, sets `Generated` and `dirty`, records the path for auto-save, and increments `batch.done`. Failed results set `Error` and a message. Cancel stops before the next image; unprocessed items return to `Pending`. `BatchFinished` clears the AI batch slot, then immediately starts the sequential save for every successful path when ExifTool is available; otherwise it reports that metadata was not saved.

### 8.4 Save flow

```
save_paths(paths)
  ├─ used by Save Selected and by automatic post-AI saving
  ├─ reject if batch active / no ExifTool
  ├─ items with has_metadata() (title non-empty)
  ├─ BatchProgress { saving: true }
  └─ spawn sequential loop
       ├─ optional backup_original → .autowording-backups/<name>
       ├─ spawn_blocking(ExifTool::write)
       ├─ optional ExifTool::read + verify_written
       ├─ send SaveDone (Saved, dirty=false, or error message)
       └─ send BatchSaved; clear batch slot
```

Saves are deliberately sequential to avoid launching many ExifTool processes over the same folder. Automatic post-AI saving reuses this exact loop.

### 8.5 Metadata load flow

```
selection change / open folder / ExifTool ready
  └─ request_metadata(path) for selected paths
       ├─ skip if no tool or already requested
       ├─ spawn_blocking(ExifTool::read)
       └─ send MetaLoaded or MetaFailed
            ├─ set meta_loaded; flag read errors
            ├─ store GPS location and capture date
            ├─ fill dims if absent
            └─ populate title/caption/keywords only if all empty AND status Pending
```

`meta_requested` prevents duplicate reads. It is cleared on folder refresh/open and when ExifTool becomes ready.

### 8.6 Event reference (`src/state.rs::WorkerEvent`)

| Event | Payload | UI handling |
| --- | --- | --- |
| `Thumbnail` | path, `ColorImage` | create texture, cache |
| `Preview` | path, `ColorImage` | replace preview texture |
| `ImageFailed` | path, error | mark cache failure + item message |
| `AiStarted` | path | `Queued → Processing` |
| `AiDone` | path, `Result<GeneratedMetadata, String>` | fill fields or set `Error`; progress++ |
| `BatchFinished` | ok, failed | clear AI batch; auto-save successful paths or toast |
| `SaveDone` | path, `Result<(), String>` | `Saved`/message; progress++ |
| `BatchSaved` | ok, failed | clear batch; toast |
| `MetaLoaded` | path, `ExistingMetadata` | fill empty pending item |
| `ModelsLoaded` | provider, endpoint, `Result<Vec<String>, String>` | populate model dropdown or inline error; ignore stale endpoint/provider results |
| `ExifToolReady` | `Option<ExifTool>`, status | store tool; retry metadata for the selection |

### 8.7 Core data structures

```rust
enum Status {
    Pending, Queued, Processing, Generated, Saved, Error,
}

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

struct Selection {
    selected: HashSet<PathBuf>,
    anchor: Option<PathBuf>,
    active: Option<PathBuf>,
}

struct BatchProgress { total: usize, done: usize, saving: bool }

struct GeneratedMetadata { title, caption, keywords, editorial }
struct Metadata { title, caption, keywords }       // written to disk
struct ExistingMetadata {
    title: Option<String>, caption: Option<String>,
    keywords: Vec<String>, dims: Option<(u32, u32)>,
}
```

`App` keeps `items: HashMap<PathBuf, ImageItem>` for status/metadata persistence, `folder_images: Vec<PathBuf>` for grid order, and one `Selection`.

---

## 9. Source map

```
Cargo.toml                     crate metadata, dependencies, release profile
Cargo.lock                     locked dependency graph (committed)
build.bat                      GNU-toolchain build wrapper
README.md                      this document

src/main.rs                    Windows subsystem attr, egui window setup
src/app.rs                     controller: folders, selection, workers, batches, shortcuts
src/state.rs                   ImageItem/Status/Selection, folder scan, decode, events
src/config.rs                  Provider, ProviderSettings, Config load/save/defaults
src/theme.rs                   colors, egui style, Segoe UI loading, card frame
src/cache/mod.rs               cache module root + ThumbnailCache re-export
src/cache/thumbnails.rs        texture cache, in-flight/failure sets, lifecycle test

src/ui/mod.rs                  menu bar, mode toggle, status bar, widgets, about, drop overlay
src/ui/browser.rs              folder panel + multi-select grid + active preview
src/ui/viewer.rs               large viewer + navigation + filmstrip
src/ui/metadata_panel.rs       single inspector, batch inspector, AI/EXIF settings
src/ui/tree.rs                 lazy system folder tree: drives, Quick access, reveal, expansion tests

src/ai/mod.rs                  AiRequest, prepare_image, provider dispatch, endpoint, snippet
src/ai/prompt.rs               exact prompt, tolerant parser, keyword normalizer, validation
src/ai/ollama.rs               Ollama payload/chat
src/ai/lmstudio.rs             LM Studio payload, endpoint normalization, chat
src/ai/models.rs               provider model-list discovery, endpoint normalization, parsing
src/ai/openai.rs               OpenAI-compatible payload/chat
src/ai/anthropic.rs            Anthropic Messages payload/chat
src/ai/tests.rs                hand-written TCP mock HTTP server tests for all providers and model lists

src/metadata/mod.rs            ExifTool detection, argfile protocol, read/write, round-trips
src/metadata/tags.rs           read tag list, exiftool JSON parser, read tests
src/metadata/writer.rs         Metadata type, tag-writing argument builder, writer tests
```

Dependency roles:

| Crate | Role |
| --- | --- |
| `eframe`/`egui` 0.28.1 | Native window, OpenGL rendering, widgets |
| `tokio` | Runtime, channels, spawn_blocking |
| `reqwest` 0.12 (`json`, `native-tls`) | Provider HTTP (Schannel on Windows) |
| `serde` / `serde_json` | Config and provider JSON |
| `toml` | Config persistence |
| `directories` 5 | OS config directory |
| `rfd` 0.14 | Native file/folder dialogs |
| `walkdir` 2.5 | Recursive add/drop scan |
| `base64` 0.22 | Image payload encoding |
| `regex` 1.10 | Tolerant field parser |
| `chrono` 0.4 (`clock`) | Local editorial date |
| `anyhow` 1 | Error context |
| `image` 0.25 (`jpeg` only) | Decode, resize, re-encode |

---

## 10. Selection engine

`Selection` is intentionally independent of egui so it can be tested without a window.

### 10.1 Rules

- `selected`: path set.
- `anchor`: starting point for Shift ranges; normally the last plain/Ctrl-click.
- `active`: image shown in preview/Viewer; always selected after a click.
- `primary(ordered)`: returns active if it is still selected; otherwise the first selected path in grid order.

### 10.2 Behavior

| Gesture | Result |
| --- | --- |
| Plain click | Clear, select target, active = anchor = target |
| Ctrl+Click selected | Deselect target; active moves to first remaining selected path |
| Ctrl+Click unselected | Add target; active = anchor = target |
| Shift+Click | Clear (unless Ctrl held), select range anchor↔target, active = target; anchor stays |
| Ctrl+Shift+Click | Add range to current selection, active = target |
| Checkbox | Toggle target without clearing; active = target when selecting, first remaining when deselecting |
| Ctrl+A | Select all grid paths; anchor = first; active preserved if still valid, else first |
| Retain after folder change | Drop paths not present; repair anchor/active; active falls back to first selected |

`app.rs` calls `after_selection_change`, which requests the preview for the active path and metadata for every selected path.

---

## 11. AI subsystem

### 11.1 Request contract (`src/ai/mod.rs`)

```rust
struct AiRequest {
    provider: Provider,
    settings: ProviderSettings, // base_url, model, api_key
    image_base64: String,       // no data: prefix
    existing: ExistingMetadata, // title, caption, keywords, GPS location, capture date
    context: Option<String>,    // trimmed; omitted when empty
    global_prompt: String,      // extra system instructions
    date: String,               // e.g. "November 5, 2026"
    keyword_target: usize,      // desired keywords per image, clamped 1..=50
    title_range: (usize, usize), // configured min/max words, clamped 1..=20
    caption_range: (usize, usize), // configured min/max characters, clamped 10..=1000
}
```

Validation order:

1. Model non-empty.
2. Cloud provider API key non-empty.
3. Image base64 non-empty.
4. Build `reqwest::Client` with **300-second timeout**.
5. Dispatch to provider.
6. Parse with `prompt::parse_response_allow_empty_keywords`; normalization removes exact duplicates and singular/plural pairs (keeping the singular), then trims to the target.
7. If the request fails or the reply cannot be parsed, retry up to three times (300, 600, 900 ms apart). A parse failure reuses a repair prompt that restates the four labelled lines and includes the previous reply.
8. If the title/caption are outside their configured ranges, or the parsed reply has fewer than the target keywords, make one corrective request.
9. If the corrective reply satisfies both title and caption ranges, replace the first title/caption (and editorial flag). Always merge retry keywords into the first list, preserving order and dropping duplicates/singular-plural pairs before capping at the target.

No streaming or cancellation. Excess keywords are trimmed before the result reaches the UI.

### 11.2 Image preparation

- `image::open` (JPEG only in this build).
- Downscale longest side to **1600 px** with `FilterType::Triangle` if larger.
- Always convert to RGB8 and re-encode JPEG quality **85**.
- Base64 standard encoding, no `data:` prefix.
- Strips all metadata before transmission.
- EXIF orientation is **not** applied; portrait photos with orientation tags may be analyzed sideways.

### 11.3 Exact system prompt

`SYSTEM_TEMPLATE` in `src/ai/prompt.rs`; `{DATE}` is replaced with `Month D, YYYY` (English month, no leading zero), `{KEYWORD_TARGET}` with the configured 1–50 target, `{TITLE_MIN_WORDS}`/`{TITLE_MAX_WORDS}` with the configured title range, and `{CAPTION_MIN_CHARS}`/`{CAPTION_MAX_CHARS}` with the configured caption range. If `global_prompt` is non-empty:

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
   - Length: Between {TITLE_MIN_WORDS} and {TITLE_MAX_WORDS} words.
   - Content: Clear, descriptive summary of the primary subject.

2. Caption:
   - Length: Strict limit of {CAPTION_MIN_CHARS} to {CAPTION_MAX_CHARS} characters (including spaces).
   - Content: Factual description of the scene (or Shutterstock editorial format if applicable).

3. Keywords:
   - Quantity: Exactly {KEYWORD_TARGET} keywords.
   - Format: Single words only (no multi-word phrases/keyphrases).
   - Separator: Comma-separated.
   - Rules: Alphabetic characters only. No numbers, punctuation, or special characters.
   - Duplicates: Do not include a plural when its singular form is also present. Keep the singular (for example, keep "apple" and omit "apples").

Required Output Schema:
Title: [{TITLE_MIN_WORDS}-{TITLE_MAX_WORDS} words]
Editorial Status: [Commercial / Editorial]
Caption: [{CAPTION_MIN_CHARS}-{CAPTION_MAX_CHARS} characters]
Keywords: [word1, word2, word3, ... word{KEYWORD_TARGET}]

Reply with the four labelled lines above and nothing else. Do not use markdown.
```

Date is computed once per batch:
```rust
let now = chrono::Local::now();
let date = format!("{} {}, {}", now.format("%B"), now.day(), now.year());
```

### 11.4 User prompt

Existing metadata is included before context. Empty fields are omitted; keywords are comma-separated:

```text
Existing file metadata (reference only; the returned title, caption, and keywords replace these values):
GPS location: ...
Capture date: ...
Title: ...
Caption: ...
Keywords: ...

Use these details as context and return fresh title, caption, and keywords.
```

Context is trimmed; blank = absent:

```text
Situational context supplied by the photographer (treat it as ground truth):
<context>

Analyze the attached image and return the four labelled fields exactly as specified.
```

Without metadata or context, the prompt is only the final analyze line.

If normalization leaves fewer than the target keywords, a second request appends:

```text
The previous reply contained only N usable keywords, but exactly TARGET are required. Return the four labelled fields again, keeping the same title and caption. Provide exactly TARGET unique single-word keywords. Do not include both a singular and its plural; keep the singular. Previous keywords: ...
```

### 11.5 Provider wire formats

| Provider | Default base URL | Final endpoint | Auth | Image field | Sampling | Response |
| --- | --- | --- | --- | --- | --- | --- |
| Ollama | `http://localhost:11434` | `/api/chat` (base `/api` → `/api/chat`) | none | `messages[1].images = [base64]` | `stream:false`, temp 0.2 | `message.content`, fallback `response`; checks `error` |
| LM Studio | `http://localhost:1234/v1` | full `/chat/completions`, or `/v1` + `/chat/completions`, or bare + `/v1/chat/completions` | none | OpenAI `image_url` data URI | temp 0.2, max 900 | `choices[0].message.content` |
| OpenAI | `https://api.openai.com/v1` | `/chat/completions` | `Authorization: Bearer` | `image_url` data URI | temp 0.2, max 900 | content string, or text parts joined by `\n` |
| Anthropic | `https://api.anthropic.com/v1` | `/messages` | `x-api-key`, `anthropic-version: 2023-06-01` | base64 image block, `image/jpeg` | temp 0.2, max 900; system top-level | text blocks joined by `\n` |

`endpoint(base, path)` trims whitespace/trailing slash; empty base returns the path; exact suffix returns base unchanged; `/api` + `/api/chat` special case; otherwise concatenates.

HTTP errors include provider-specific hints for local servers, a 400-character snippet of the body, and JSON parse context. Missing expected fields are errors.

### 11.6 Tolerant reply parser

Accepts:

- Start of line/whitespace/comma/semicolon before a label.
- Optional numbering (`1.`, `2)`).
- Optional markdown (`*`, `#`, `>`, `-`).
- Labels: `Title`, `Editorial Status`, `Editorial`, `Caption`, `Description`, `Keywords`, `Keyword`.
- Optional `*` after the label; ASCII `:` or full-width `：`.

Values run to the next label, so captions/keywords may span lines. Cleaning trims whitespace/`*`/backticks/underscores, collapses whitespace, removes wrapping quotes, and strips a trailing title period.

Duplicate labels: last wins. Missing title/caption/keywords: error listing the missing fields plus a 400-character raw snippet. Editorial defaults to Commercial; the word `commercial` anywhere wins over `editorial`; otherwise `editorial` presence sets true.

### 11.7 Keyword normalization

Shared by AI parsing, the single inspector's **Normalize**, and batch append/replace:

1. Split on `, ; newline CR | /`.
2. Split each chunk on whitespace.
3. Keep only alphabetic characters (`char::is_alphabetic`; Unicode letters survive).
4. Drop results shorter than 2 characters.
5. Deduplicate case-insensitively, preserving first spelling/order.
6. Cap at the configured target (default 50).

Examples: `2024` → dropped; `3d` → dropped; `blue-sky` → `bluesky`; `New York` → `New`, `York`; `Café`/`北京` preserved.

### 11.8 Validation (advisory)

- Title word count outside the configured range (default 3–5).
- Caption character count outside the configured range (default 50–180, `chars().count()`).
- Keyword count not exactly the configured target (default 50).
- First keyword under 2 characters or containing non-alphabetic characters.

Warnings never block generation or saving.

### 11.9 Model discovery

`AI Settings` fetches the list of model IDs from the selected provider's endpoint, rather than requiring the ID to be typed from memory.

| Provider | Model-list request | Authentication | Parsed field |
| --- | --- | --- | --- |
| Ollama | `GET /api/tags` | none | `models[].name` (fallback `model`) |
| LM Studio | `GET /v1/models` | none | `data[].id` |
| OpenAI | `GET /v1/models` | `Authorization: Bearer <key>` | `data[].id` |
| Anthropic | `GET /v1/models` | `x-api-key`, `anthropic-version: 2023-06-01` | `data[].id` |

Endpoint normalization accepts the provider defaults plus common user variants: Ollama base `/api` or `/api/chat` maps to `/api/tags`; LM Studio bare host, `/v1`, or `/v1/chat/completions` maps to `/v1/models`; OpenAI/Anthropic full chat/messages URLs map back to `/v1/models`.

The request has a 20-second timeout. HTTP/JSON errors include the provider and a body snippet. Cloud providers reject discovery without a key before sending a request. Results are sorted case-insensitively and deduplicated. A configured real model missing from the response is inserted into the dropdown so it can still be selected. The retired LM Studio `local-model` placeholder is never inserted; when LM Studio has no model configured, the first endpoint model is selected. Discovery is asynchronous through `WorkerEvent::ModelsLoaded`; stale responses from an old provider/endpoint are ignored. The `Fetch` button retries after an error.

---

## 12. Metadata subsystem

### 12.1 Tag mapping

| Field | Tags written |
| --- | --- |
| Title | `XMP-dc:Title`, `IPTC:ObjectName`, `EXIF:XPTitle` |
| Caption | `EXIF:ImageDescription`, `IPTC:Caption-Abstract`, `XMP-dc:Description` |
| Keywords | repeating `IPTC:Keywords`, repeating `XMP-dc:Subject` |

- Caption empty + title present → only `EXIF:ImageDescription=title`.
- Title empty → no title tags.
- Both empty → no descriptive tags.
- Keyword list empty → no keyword arguments, so existing file keywords remain. The UI cannot clear keywords.
- `writer::write_args` caps keywords at 50.

### 12.2 ExifTool detection order

1. Configured `exiftool_path`:
   - directory → `<dir>\exiftool.exe`, `<dir>\exiftool`
   - file → as-is
2. `<exe>\exiftool.exe`
3. `<exe>\resources\exiftool\exiftool.exe`
4. `<exe>\resources\exiftool.exe`
5. `<cwd>\resources\exiftool\exiftool.exe`
6. `<cwd>\resources\exiftool.exe`
7. `exiftool` from `PATH`

A candidate is accepted only if `exiftool -ver` succeeds and prints a non-empty version. Detection runs in `spawn_blocking`; the version appears in the status bar and EXIF settings.

### 12.3 Invocation protocol

Every read/write:

1. Build `Vec<String>` arguments.
2. Write them UTF-8 to `%TEMP%\autowording-exif-<pid>-<counter>.args`, one argument per line.
3. Flatten `\r`/`\n` inside values to spaces.
4. Execute `<exiftool> -charset filename=utf8 -@ <argfile>`.
5. Remove the argfile after the process finishes, success or failure.
6. On Windows set `CREATE_NO_WINDOW` (0x0800_0000).

Why: Windows converts `argv` to the process ANSI code page, mangling `Café`; a UTF-8 argfile preserves accented/CJK values and file names. `-charset filename=utf8` tells ExifTool the argfile names are UTF-8. Do not reintroduce direct `Command::arg` for tag values/paths.

### 12.4 Write arguments

Base flags, always:

```text
-P -overwrite_original -charset iptc=utf8 -m
```

- `-P` preserves file modification time.
- `-overwrite_original` overwrites in place; no `_original` backups.
- `-charset iptc=utf8` writes IPTC as UTF-8.
- `-m` ignores minor metadata errors.

Keyword rule: every keyword uses its own `=` assignment:

```text
-IPTC:Keywords=bridge
-IPTC:Keywords=sunrise
-XMP-dc:Subject=bridge
-XMP-dc:Subject=sunrise
```

ExifTool treats `=` like `+=` if the same tag is also set with `+=`/`-=` in the same command. Pure repeated `=` replaces the list. `overwriting_metadata_replaces_previous_keywords` verifies this against real ExifTool.

### 12.5 Read arguments and precedence

```text
-j -charset iptc=utf8
-Title -ObjectName -Description -Caption-Abstract -ImageDescription
-XPTitle -IPTC:Keywords -Subject -ImageSize
<path>
```

`parse_read_json`:

- Requires a JSON array with at least one object.
- Title: `Title` → `XPTitle` → `ObjectName`; empty/whitespace falls through.
- Caption: `Description` → `Caption-Abstract` → `ImageDescription`.
- Keywords: merge `Keywords` then `Subject`; array or comma-separated string; trim/drop empty; case-insensitive dedupe.
- `ImageSize` only lowercase `WIDTHxHEIGHT`; uppercase `X` rejected.
- Invalid JSON/empty array/object instead of array = error.
- Wrong-type values are ignored.
- Output decoded with `from_utf8_lossy` before JSON parsing.

### 12.6 Errors

- `write` rejects a path that is not an existing file.
- `read` lets ExifTool report missing files.
- Non-zero exit is an error; write errors prefer stderr, then stdout, trimmed.
- Success is exit status only; "1 image files updated" is not parsed.
- Each file is a separate ExifTool process; no directory batching.
- Argfile write failure and spawn failure are surfaced with context.

---

## 13. Configuration

### 13.1 Location

```rust
directories::ProjectDirs::from("", "", "AutoWording")
    .config_dir()
    .join("config.toml")
```

On Windows this resolves to:

```text
C:\Users\<user>\AppData\Roaming\AutoWording\config\config.toml
```

`ProjectDirs::config_dir()` appends `config`. The settings panel shows the exact path. Fallbacks: executable directory, then relative `config.toml`.

`Config::load`:

- Reads if present.
- On parse error, logs to stderr and uses **all defaults**.
- On missing file, uses defaults and immediately saves.
- Sets `path` (serde-skipped, never written).

`Config::save` creates parent directories and writes pretty TOML. UI debounces 500 ms.

### 13.2 Schema and defaults

```rust
enum Provider { Ollama, LmStudio, OpenAi, Anthropic } // serde: snake_case
struct ProviderSettings { base_url, model, api_key }
struct Config {
    provider,
    ollama, lmstudio, openai, anthropic: ProviderSettings,
    global_prompt,
    exiftool_path,
    last_folder,        // reopened at startup
    theme,              // system | light | dark
    density,            // comfortable | compact
    marketplace,        // general | adobe_stock | shutterstock | istock | dreamstime | alamy | depositphotos | pond5
    keyword_target,     // desired keywords per image, clamped 1..=50 at use
    title_min_words,    // title minimum words, clamped 1..=20 at use
    title_max_words,    // title maximum words, clamped 1..=20 at use
    caption_min_chars,  // caption minimum characters, clamped 10..=1000 at use
    caption_max_chars,  // caption maximum characters, clamped 10..=1000 at use
    backup_originals,   // copy into .autowording-backups before the first write
    verify_writes,      // read back and compare after each write
    #[serde(skip)] path,
}
```

| Field | Default |
| --- | --- |
| `provider` | `ollama` |
| `ollama.base_url` / `model` | `http://localhost:11434` / `llava` |
| `lmstudio.base_url` / `model` | `http://localhost:1234/v1` / empty (first fetched model is selected) |
| `openai.base_url` / `model` | `https://api.openai.com/v1` / `gpt-4o-mini` |
| `anthropic.base_url` / `model` | `https://api.anthropic.com/v1` / `claude-sonnet-4-20250514` |
| all `api_key` | empty |
| `global_prompt` | empty |
| `exiftool_path` | empty (auto-detect) |
| `last_folder` | empty |
| `theme` | `system` (follows the Windows app theme) |
| `density` | `comfortable` |
| `marketplace` | `general` |
| `keyword_target` | `50` |
| `title_min_words` / `title_max_words` | `3` / `5` |
| `caption_min_chars` / `caption_max_chars` | `50` / `180` |
| `backup_originals` | `true` |
| `verify_writes` | `true` |

TOML provider values: `ollama`, `lm_studio`, `open_ai`, `anthropic`; theme values: `system`, `light`, `dark`; density values: `comfortable`, `compact`. Unknown values fail the whole parse and fall back to defaults. `#[serde(default)]` fills missing fields/sections.

```toml
provider = "open_ai"
global_prompt = "Prefer British spelling and avoid brand names."
exiftool_path = ""
last_folder = "C:\\Photos\\2026_Stock"
theme = "system"
density = "comfortable"
marketplace = "general"
keyword_target = 50
title_min_words = 3
title_max_words = 5
caption_min_chars = 50
caption_max_chars = 180
backup_originals = true
verify_writes = true

[ollama]
base_url = "http://localhost:11434"
model = "llava"
api_key = ""

[lmstudio]
base_url = "http://localhost:1234/v1"
model = ""
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

The marketplace value clamps the three output limits at use time (prompt, validation chips, slider maxima). A hand-edited `keyword_target = 50` with `marketplace = "adobe_stock"` is used as 49; an Alamy caption range with `caption_max_chars = 400` is used as 150. `Config::apply_marketplace` writes the profile's defaults into those fields when the user switches profiles.

### 13.3 Security

- API keys are plaintext; the UI masks the field only.
- `config.toml` is outside the repo. Never commit, print, or log it.
- No telemetry, analytics, auto-update, or crash reporting.
- Only the selected provider endpoint receives network traffic.
- Images are re-encoded/downscaled before transmission, stripping metadata.
- ExifTool runs locally with user permissions.
- Saving is destructive: `-overwrite_original`.

---

## 14. Tests

### 14.1 Run

```bat
build.bat test
```

or:

```bash
SELF="$HOME/.rustup/toolchains/stable-x86_64-pc-windows-gnu/lib/rustlib/x86_64-pc-windows-gnu/bin/self-contained"
export PATH="$PWD/.tools/gnu-bin:$SELF:$PATH"
cargo +stable-x86_64-pc-windows-gnu test
```

Expected: **127 passed; 0 failed; 0 ignored**.

List: `cargo test -- --list`.

### 14.2 Inventory

| Module | Count | Coverage |
| --- | --- | --- |
| `ai::prompt::tests` | 30 | markdown/CRLF/numbered/full-width labels, duplicate labels, multiline values, editorial precedence, title cleanup, keyword normalization/Unicode/dedupe/cap, singular/plural de-duplication, range-aware prompt/validation/trimming, retry prompt, unparsable-reply repair prompt, validation boundaries, exact editorial caption, existing metadata/context prompt |
| `ai::tests` | 27 | mock HTTP sequence server for all providers; model-list endpoints, auth headers, parsing and no-key validation; payload/header/endpoint assertions; malformed JSON; HTTP errors; missing fields; image resize/encoding/rejection; snippet truncation; short-keyword, title/caption, and malformed-reply retries |
| `config::tests` | 13 | defaults, serde names, accessors, save/load round-trip incl. `last_folder`/`theme`/`density`/`keyword_target`/ranges, target and range clamping, marketplace profile caps/defaults and profile switching, legacy LM Studio placeholder cleanup, invalid TOML, path stability |
| `metadata::tags::tests` | 8 | read precedence/fallbacks/merge/dedupe/invalid payloads/string keywords, GPS and capture-date parsing |
| `metadata::writer::tests` | 7 | all tags, title fallback, empty metadata, no `+=`, Unicode, 50-keyword safety cap |
| `metadata::tests` | 10 | real ExifTool round-trip and write verification, safe backup preservation, overwrite semantics, 50 keywords, Unicode values, Unicode file names, missing files, explicit/directory detection, argfile formatting |
| `state::tests` | 24 | extension/folder scans, decode resize/corrupt, item conversion, field-lock merging, progress, selection math, Ctrl/Shift/checkbox/toggle/retain/clear, aggregate common/mixed, keyword append, singular/plural append, browser text/status filtering and name/status/keyword sorting |
| `cache::thumbnails::tests` | 1 | request lifecycle blocks duplicates/failures |
| `theme::tests` | 2 | Windows theme registry parsing, palette application |
| `ui::tree::tests` | 4 | sorted children/expansion/depth; refresh preserves expansion and rereads children; reveal expands only the ancestor chain; breadcrumb path segments |
| `ui::browser::tests` | 1 | selection-summary percentages for empty, partial, full, and over-target selections |

ExifTool integration tests skip with `exiftool not available; skipping test` if detection fails. This keeps the suite runnable without ExifTool, but those eight only prove metadata correctness when ExifTool is installed. On this machine they run.

Provider tests make no real API calls. `src/ai/tests.rs` has a TCP HTTP server on `127.0.0.1:0` that can return a sequence of responses; it reads each request, asserts method/path/headers/body, and returns canned JSON. The keyword, title/caption, and malformed-reply retry tests use two responses; the give-up test consumes all four (the first request plus three parse retries).

Temporary artifacts: `%TEMP%\autowording-tests`, `autowording-state-tests`, `autowording-config-tests`, `autowording-tree-tests`.

### 14.3 Not tested

- egui rendering, widget interaction, drag-and-drop, shortcut dispatch, and visual layout (no UI automation).
- Live Ollama/LM Studio/OpenAI/Anthropic calls.
- Windows native file dialogs (`rfd`).
- Real folder volumes, permission errors, junctions/symlink loops.
- ExifTool behavior on corrupt JPEGs.
- Interactive pause/cancel behavior and process interruption; cancellation is implemented between images but is not UI-automated.
- Memory behavior with thousands of images.
- Non-Windows builds.

---

## 15. Invariants and gotchas

1. **Repeated `=` replaces; any `+=` makes `=` behave like append.** Do not mix.
2. **UTF-8 argfile is mandatory on Windows.** Direct `argv` loses accents/CJK.
3. **`-overwrite_original` is irreversible.** Backups are opt-out via `backup_originals`; there is still no undo.
4. **Metadata only loads into empty `Pending` items.** Prevents clobbering generation/user edits.
5. **Generation replaces all four fields unless locked**, including manual edits.
6. **Save requires a non-empty title.** Caption-only/keyword-only items are skipped.
7. **Validation never blocks save.**
8. **One batch at a time**, shared by AI and save. A stuck provider can hold it for up to 300 s.
9. **AI generation and saves are sequential.** `batch.done` increments in `AiDone` only when `!saving`.
10. **Selection is by path, not index.** Grid reorder/reload must not corrupt it.
11. **Shift anchor persists** across Shift clicks; Ctrl+Shift extends without clearing.
12. **`primary()` falls back to the first selected path** if the active path is gone.
13. **Config parse failure resets everything in memory**, not field-by-field.
14. **The config path contains an extra `config` directory** on Windows.
15. **Anthropic uses `x-api-key`; OpenAI uses Bearer;** local providers send no auth.
16. **Successful generation auto-saves after the AI batch.** ExifTool is required; without it, results remain `Generated` and are not written.
17. **LM Studio endpoint normalization** accepts host, `/v1`, or full `/chat/completions`.
18. **Ollama base ending `/api`** becomes `/api/chat`.
19. **Provider failures retry automatically.** Each image gets one request plus up to three retries (300/600/900 ms) for transport errors and unparsable replies; the range/keyword corrective pass is a separate final request. Items that still fail become `Error` and can be retried via **Retry failed**.
20. **The model sees a re-encoded JPEG**, quality 85, max 1600 px; fine detail may be lost.
21. **No EXIF orientation correction.**
22. **Keywords are single words by design**; multi-word concepts split.
23. **Keyword dedupe preserves first spelling**, case-insensitive.
24. **Unicode keywords pass validation** (`is_alphabetic`), despite the prompt asking for alphabetic characters.
25. **Editorial toggle does not rewrite the caption.**
26. **Folder scans are non-recursive for the grid**, recursive for dropped folders/add paths.
27. **The left tree is rooted at the system, not the opened folder.** Mounted drives and Quick access stay available; opening a folder only reveals its ancestor chain.
28. **`folder_root` is separate from `current_folder`**: `folder_root` is the opened folder's volume root; the current folder is the selection.
29. **Refresh keeps existing selections and leaves an empty selection empty.**
30. **`meta_requested` is cleared on folder open/refresh/ExifTool-ready**, causing re-reads.
31. **The app is not code-signed**; SmartScreen may warn.
32. **No installer, auto-update, single-instance enforcement, or tray icon.**
33. **Manual edits made after generation are not auto-saved**; generation auto-saves once.

---

## 16. Extending safely

### 16.1 Add a provider

1. Add a `Provider` variant in `config.rs`.
2. Update `ALL`, `label`, `is_cloud`, `requires_key`, `default_base_url`, `default_model`, `base_hint`, `model_hint`, `guidance`.
3. Add a `ProviderSettings` field, default, and `settings`/`settings_mut` arms.
4. Create `src/ai/<provider>.rs` with testable `payload`/`chat`.
5. Declare it in `ai/mod.rs` and add a `generate` match arm.
6. Add mock-server tests and config round-trip coverage.
7. Update this README's provider table/defaults.

Exhaustive matches force most updates at compile time.

### 16.2 Change the prompt/schema

1. Edit `SYSTEM_TEMPLATE`/`system_prompt`/`user_prompt`.
2. Update `GeneratedMetadata`, `parse_fields`, `parse_response`, missing-field errors.
3. Update `validate`, the single inspector chips, and batch aggregate fields.
4. Update metadata writer/state if the new field must persist.
5. Add parser tests for old and new formats.
6. Update §11.3 verbatim.

### 16.3 Add/change a metadata tag

1. Update `Metadata`/`ExistingMetadata`.
2. Update `writer::write_args` (never mix `=`/`+=`).
3. Update `READ_TAGS` and `parse_read_json` precedence.
4. Update `ImageItem`, `to_metadata`, inspector UI, and batch aggregation if needed.
5. Add tag-argument and real round-trip tests.
6. Update §12.1.

### 16.4 Change selection behavior

Edit `Selection` in `state.rs`, then update `state::tests` for plain/Ctrl/Shift/Ctrl+Shift/checkbox/retain. The UI only converts egui modifiers into `SelectModifiers`; keep that translation thin.

### 16.5 Change batch processing

`generate_paths` runs jobs in order on one async loop: send `AiStarted`, prepare the image, call `ai::generate`, send `AiDone`, then start the next image. Images waiting for their turn stay `Queued`. Preserve that distinction if refactoring.

### 16.6 Add UI automation

There are no test hooks or headless mode. You would need to separate pure state transitions from egui rendering (e.g. an `AppCore` event-in/out API) or drive the real window with Windows UI automation. Do not claim UI coverage without that work.

---

## 17. Troubleshooting

| Symptom | Likely cause / fix |
| --- | --- |
| `ExifTool not found` | Install it, add to `PATH`, or set the path in **AI Settings → EXIF writer**, then **Re-check**. |
| Save disabled | No ExifTool, no selection, no title, or a batch is running. |
| Grid empty | The folder has no direct `.jpg/.jpeg/.jpe/.jfif` files; use **Add images** or select a different folder. |
| Tree does not show a subfolder | It starts with `.` or is one of the skipped names; refresh after creating it. |
| `Ctrl+A` types in a field instead of selecting | Shortcuts are suppressed while a widget has focus; click the grid first. |
| Batch buttons disabled | A generation/save batch is already running. |
| Keywords not replaced | Check `writer::write_args` for `+=`. |
| Accented text becomes ASCII | Arguments bypassed the UTF-8 argfile. |
| OpenAI/Anthropic 401 | Missing/incorrect API key; keys are per provider. |
| Model list is empty or fails | Check the endpoint/port and click **Fetch**; cloud providers need an API key. A failed discovery leaves a manually editable model field. |
| Ollama/LM Studio connection refused | Start the local server; check endpoint/port. |
| LM Studio 404 `/v1/v1/...` | Verify `lmstudio::endpoint_for` still normalizes. |
| Model reply unparseable | Error includes a 400-char snippet; use a stronger instruction-following vision model. |
| Metadata visible in one app, not another | Readers prioritize different families; verify with `exiftool -a -G1 -s file.jpg`. |
| Build `kernel32.lib` / `link.exe` errors | Use `build.bat` or install the Windows SDK and fix `link.exe` order. |
| `ring` build failure | `reqwest` uses `native-tls`; do not re-enable rustls without a C compiler. |
| Unicode filename not found | Ensure `-charset filename=utf8` and argfile path handling remain intact. |
| Tests skip metadata cases | ExifTool not detected; check stderr and install it. |

Diagnostic:

```bat
exiftool -a -G1 -s "path\to\image.jpg"
```

---

## 18. Limitations and non-goals

- Windows-only; no macOS/Linux packaging or testing.
- JPEG only; no PNG/TIFF/HEIC/RAW/video.
- Writes into the JPEG; no XMP sidecars.
- No per-image cancel; Pause/Cancel stop between images.
- No automatic retries/backoff/rate-limit handling/cost accounting.
- No EXIF orientation correction before AI.
- No undo, metadata diff, or raw ExifTool output viewer.
- No keyword clearing when the list is empty.
- No keyword translation/synonym expansion/per-market limits.
- Queue/selection are in memory only; only config persists.
- Folder grid does not show subfolder images recursively.
- No installer, signing, auto-update, tray icon, single-instance enforcement.
- No CI; tests run manually.
- No license file. Add one before redistribution (default is all rights reserved).

---

## 19. Glossary

| Term | Meaning |
| --- | --- |
| **ACDSee layout** | Browser mode with folder tree + thumbnail grid + preview/inspector; Viewer mode for a single large image. |
| **Microstock** | Low-cost royalty-free/editorial stock marketplaces. |
| **Editorial** | Newsworthy/documentary content; recognizable people/events usually need releases for commercial use. |
| **Commercial** | Advertising-usable content. |
| **IPTC** | News metadata: `Keywords`, `ObjectName`, `Caption-Abstract`. |
| **XMP** | Adobe XML metadata: `dc:title`, `dc:description`, `dc:subject`. |
| **EXIF** | Camera/image metadata: `ImageDescription`, `XPTitle`. |
| **ExifTool** | Phil Harvey's Perl metadata CLI used here. |
| **Argfile** | UTF-8 file passed with `-@`, one ExifTool argument per line. |
| **Shutterstock caption format** | `CITY, STATE/COUNTRY – MONTH DAY, YEAR: Factual sentence.` |
| **Active image** | Primary focus shown in preview/Viewer; may be part of a larger selection. |
| **Anchor** | Starting image for Shift+Click range selection. |
| **Dirty** | Generated/saved metadata was edited and needs saving. |

---

## 20. Verification record

Last verified on the development machine (Windows 11, 2026-11-06):

- `cargo +stable-x86_64-pc-windows-gnu test` → **127 passed, 0 failed** (includes real ExifTool round-trips).
- `cargo +stable-x86_64-pc-windows-gnu build --release` → success, no warnings; `target/release/autowording.exe` ~21 MB.
- The redesigned release was smoke-launched; the native window `AutoWording` was created and responding.
- The UI was **not** visually inspected or driven through folder/grid/batch interactions in this pass; selection/tree logic is covered by unit tests, but layout and click behavior remain visually unverified.
- No live generation calls were made; provider tests use mock HTTP servers. A live LM Studio `/v1/models` endpoint was reachable and returned model IDs, but the settings dropdown was not visually driven.
