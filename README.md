# AutoWording

**A native Windows 11 ACDSee-style desktop application that uses a vision LLM to generate microstock metadata (Title, Caption, adjustable title-word and caption-character ranges, and an adjustable keyword target defaulting to 50) for images, then writes it directly into JPEG/PNG/TIFF EXIF / IPTC / XMP tags. MP4/MOV files share the same browser and inspector; their title, caption, keywords, category and rights are read from and written to XMP and QuickTime Keys, together with the creator, copyright, credit and usage-terms values configured in Settings.**

| Item | Value |
| --- | --- |
| Language | Rust 2021, minimum `rust-version = 1.92` (verified with 1.99.1) |
| GUI | `gpui-kit` 0.9.0 on GPUI 0.3.11, Windows 11 light/dark appearance, DirectX 11 |
| Workspace | ACDSee Classic dual-mode: **Browser** (This PC folder tree + thumbnail grid + inspector) and **Viewer** (single image or video placeholder + filmstrip) |
| Selection | Path-based multi-selection with Ctrl+Click, Shift+Click, Ctrl+Shift+Click, and Select all |
| AI transport | `reqwest` 0.12 + `tokio` multi-thread runtime, `native-tls` (Windows Schannel) |
| Batch engine | Sequential AI processing (one image at a time), FIFO retry queue with two retries per failed image, live `Queued`/`Processing`/`Generated`/`Saved`/`Error` status labels |
| Metadata engine | ExifTool command-line executable (external dependency) |
| Platform | Windows 10/11 (build and runtime are Windows-specific) |
| Binary | `target\release\autowording.exe` (67,884,693 bytes, Windows subsystem, no console window) |
| Tests | **215** unit/integration/UI tests, all passing as of 2026-11-09 |
| State | Single crate, no workspace, no PR CI, no installer, no license file; tagged builds run in GitHub Actions |

This README is the operating manual for another LLM or developer. It records the exact prompt, wire formats, tag mappings, selection rules, batch semantics, filesystem contracts, and pitfalls that are easy to regress.

---

## 1. Agent quick contract (read before changing code)

1. **JPEG, PNG, TIFF and MP4/MOV input.** The `image` crate enables `jpeg`, `png` and `tiff`. Accepted image extensions are `.jpg`, `.jpeg`, `.jpe`, `.jfif`, `.png`, `.tif` and `.tiff`; accepted video extensions are `.mp4` and `.mov` (all case-insensitive), used by folder scans and the Add picker. Images keep EXIF/IPTC/XMP; PNG is XMP-first, TIFF carries EXIF/IPTC/XMP. MP4/MOV use a separate video path: ExifTool `-G1` reads group-qualified XMP and QuickTime Keys, and writes both XMP and Keys; video never uses IPTC/EXIF. HEIC/RAW/MPG remain unsupported.
2. **ExifTool is required for metadata reads and writes.** There is no native Rust metadata writer. Without ExifTool, AI generation works but Save is disabled and existing metadata is not loaded.
3. **Keyword replacement requires repeated `=` assignments and no `+=`.** ExifTool treats `=` like `+=` in any command that also contains `+=`/`-=`. Image `writer::write_args` MUST emit only `-IPTC:Keywords=value` and `-XMP-dc:Subject=value`; video `video_write_args` emits repeated `-XMP-dc:Subject=value` and one comma-separated `-Keys:Keywords=value`.
4. **Every ExifTool argument goes through a UTF-8 argfile.** Windows `argv` code-page conversion mangles `Café` to `Cafe`. Do not "simplify" `ExifTool::run` back to direct tag/path arguments.
5. **Saving overwrites the original media.** `-overwrite_original` is always used. The configurable `.autowording-backups` copy preserves the first pre-write version. **Undo last save** restores the in-memory metadata from before the last write (up to five writes per image or video) and marks it Modified; it does not rewrite file bytes.
6. **The prompt and tag mapping are product requirements.** The exact system prompt is in §11.3; tests assert substrings and parsed outputs. Do not reword casually.
7. **The advisory validation does not block saving.** Title-word and caption-character range checks and the keyword acceptance percentage (warning below the configured share of the target, rounded up) only produce warning labels.
8. **Selection is path-based.** `Selection` stores `HashSet<PathBuf>` plus `active` and `anchor`. Do not replace it with grid indices; statuses must survive folder reloads and reordering.
9. **The grid shows the current folder's direct images**, plus any explicitly added files, sorted case-insensitively. Folder scanning is non-recursive. `add_paths` can expand a folder recursively, but the GPUI Add button only picks individual files.
10. **Metadata loading never clobbers generated/edited content.** `MetaLoaded` fills only an empty item whose status is `Pending`. An image whose file already has a title, a caption and the configured acceptance percentage of the keyword target (rounded up) loads as `Saved`, not `Pending`; below that floor it stays `Pending` so AI can complete it.
11. **Only one batch runs at a time.** Generation and saving share `App::batch`; all batch buttons are disabled while it is `Some`.
12. **AI batches are sequential, FIFO, and pausable.** One image is prepared and generated at a time; Pause stops before the next image and Cancel stops after the current request. Once Cancel is pressed, the button becomes a disabled **Cancelling…** spinner, Pause/retry controls are disabled, and the card reports where the queue will stop until the worker finishes. A failed image moves to the back of the queue and gets up to two retries (three total attempts). Images with complete metadata, including saved files, are skipped automatically unless **Force regenerate** is enabled.
13. **Saving requires a non-empty title.** Selected items without a title are skipped and counted in a toast.
14. **AI generation saves every result immediately.** Each generated result is written through ExifTool as soon as field locks have been applied, before the next image is requested. Provider/prompt/ExifTool/folder/theme/density/output-range/keyword-target/backup/verification/rights settings persist after a 500 ms debounce.
15. **API keys are plaintext** in `config.toml`. Never commit, print, or log that file.

---

## 2. What the application does

AutoWording is a batch keywording workspace for microstock photographers. The workflow follows ACDSee Photo Studio's Browser/Viewer model:

1. Browse This PC in the left system folder tree, then open a stock folder. The center grid shows that folder's direct images and MP4/MOV videos.
2. Select one image or video (single inspector) or many (batch inspector).
3. Run AI on the selection. Images are processed one at a time: each request sends a downscaled image plus the strict prompt and the image's existing metadata (title, caption, keywords, GPS location, capture date) and session context. The active marketplace profile's title range, caption range, and keyword target are embedded in the prompt. MP4/MOV files are skipped before this step because AI video generation is not implemented.
4. Each AI result is written to its file immediately through ExifTool (title, caption, keywords and category into EXIF/IPTC/XMP, XMP for PNG, plus the active rights values), one file at a time, before the next image starts. Manual video saves use the same path and write XMP/QuickTime Keys instead. Check or edit Title, Editorial, Category, Caption and Keywords in the Metadata tab afterwards. In batch mode, apply title/caption/keyword changes across the selection from the Batch tab.
5. After manual edits, use **Save** to write the updated metadata. Successful generation results are already on disk.
6. Cards show `Pending`, `In queue`, `Processing`, `Generated`, `Saved`, `Failed`, or `Modified` labels.
7. Switch to Viewer for a large image or a `VIDEO` preview placeholder, with previous/next navigation and a filmstrip.

Cloud providers receive only the re-encoded, downscaled image. Ollama and LM Studio keep traffic local. Files are modified in place.

---

## 3. Feature summary

- GPUI command bar: AutoWording lockup; **Open**, **Add**, **Generate**, **Save**; **Browser | Viewer**; light/dark toggle; **Hide/Show panel** (Ctrl+B); and **More**. The executable embeds a blue-to-cyan **AW** icon for Explorer and the taskbar.
- Three-pane Browser: 220 px lazy Windows-style folder tree with Quick access, mounted drive roots, and refresh; 168 px thumbnail cards; 320–356 px responsive, spring-animated collapsible inspector; its collapsed state persists across launches. Browser mode has no preview pane; the large preview lives in Viewer.
- Inspector tabs: equal-width **Metadata / Batch / Settings** segmented control. Metadata is the single-image editor, Batch contains shared-field apply actions and the live queue card, and Settings holds provider, prompt, marketplace, rights metadata, appearance, ExifTool, backup, and verification controls.
- Selection: plain click, Ctrl+Click toggle, Shift+Click contiguous range, Ctrl+Shift+Click additive range, and **Select all**.
- Visual states: selected accent border, status text colors (pending, info, success, warning, danger) and hover feedback from GPUI Kit controls. Grid cards have no checkbox.
- Grid filter and sort bar: text search over file name, title, caption, category and keywords; status buttons (All, Pending, Queued, Working, Generated, Saved, Failed, Modified); a **Sort** button cycling name, status, keyword count and modified time; an Ascending/Descending toggle; a **Force regenerate** switch; and **Clear** while a filter is active. The header shows `N visible`. **Select all** and the sort/filter controls act on the filtered view.
- Orientation-correct images: the EXIF orientation tag is applied to thumbnails, previews and the AI input, with the XMP `tiff:Orientation` / `exif:Orientation` value as a fallback when EXIF is missing. Phone/camera portraits display and analyze upright, and reported dimensions are post-rotation.
- Video browsing: MP4/MOV files appear beside JPEG/PNG/TIFF in folder scans, Add, search, selection, CSV export and the inspector. Grid cards, filmstrip and Viewer show a `VIDEO / Preview unavailable` placeholder; thumbnails and playback are not generated. Title, caption, keywords, category and rights can be edited and saved to XMP + QuickTime Keys. Generate skips videos and reports that AI video generation is unavailable.
- Appearance: top-bar **Light/Dark** toggle, plus **Follow Windows / Light / Dark** buttons in Settings. The choice persists in config; System mode reads the Windows app theme at startup and whenever the preference is applied.
- Density: **Comfortable / Compact** button in the status bar (also in Settings). The preference persists in config; the GPUI layout currently uses one fixed card and field size, so the toggle does not resize the grid yet.
- Header counts: the browser header shows `N visible`; the status bar shows total images and the selected count.
- Single inspector: file name, status pill and dimensions; Title and Keywords fields; Caption textarea; Category field; per-image Creator/Copyright/Credit line/Usage terms overrides (blank fields use Settings); Lock title/caption/keywords/category checkboxes, persisted per path in config and flushed immediately on close; Editorial switch; Generate, Save to EXIF, Undo last save and Restore backup; file details with capture date, GPS and the last error.
- Batch inspector: selection count, the live queue card with a **Retry all failed** action, title/caption overwrite, keyword append/replace, find/replace, remove duplicates, normalize, sort A–Z and clear keywords with a single-step **Undo cleanup**, Generate selected, Save selected, Retry failed, and a Force regenerate switch. When images have failed, a **Why:** line groups them by the cause stored with each failure — keywords short, title length, caption length, non-English text, unreadable reply, connection, setup or image — largest group first, so the constraint costing the most images is visible without opening each one. Shared values are not aggregated into a common/mixed view.
- Sequential batch AI: one image at a time, per-image live status, FIFO failure handling, pause/resume, cancel-after-current, partial-failure reporting, retry-failed action. A failed image moves to the back of the queue and is attempted up to two more times (three total attempts). During a run the status bar shows `Generating done / total`, the failed count, a thin progress bar and an ETA; the Batch tab's queue card adds the percent, current file, the next three queued filenames plus a remaining count, Done/Waiting/Failed counts, and a **Retry all failed** action once failures exist. Images with complete metadata, including saved and generated-but-unwritten files, are skipped automatically unless **Force regenerate** is enabled; the completion toast reports how many.
- AI context: requests include the loaded title, caption, keywords, GPS location, capture date, and session context; generated title, caption, keywords and category replace the existing editable values unless the field is locked.
- Output limits in Settings: keyword target **1–50**, keyword acceptance **1–100% (default 80)**, title **1–20 words**, caption **10–1000 characters**. Title and caption are min–max ranges. A reply outside either range, or below the configured percentage (rounded up) of the keyword target, triggers one corrective request at temperature 0.7 (the first pass uses 0.5), with the previous title/caption shown verbatim and an exact target length instead of the range. A valid retry replaces an out-of-range title/caption; keywords are merged so the first result is kept and only new unique terms are appended. If the reply still fails a constraint, one escalated request runs at temperature 0.95 without the photographer's extra instructions, asking for an `Observation:` line before the five labelled fields; a reply that repeats the failed title and caption is called out in that prompt. Metadata that still fails after escalation fails the image so the batch retry queue can try again. The selected marketplace profile caps these values further (Adobe Stock: 49 keywords; Alamy: 150 caption characters).
- AI retries: each image gets one request plus up to **three retries** (300/600/900 ms apart) when the provider request fails or the reply cannot be parsed. A reply missing Title/Caption is sent back to the model with a repair instruction and the previous text; only after all attempts fail does the item become `Error` with the last parse error and raw snippet.
- Marketplace profiles: **General stock**, **Adobe Stock**, **Shutterstock**, **iStock**, **Dreamstime**, **Alamy**, **Depositphotos**, and **Pond5**, each with its own keyword, title-word, and caption-character caps and defaults. Switching a profile adopts that agency's defaults and clamps further edits; the chosen profile persists in config.
- Safe writes: optional pre-write copies in `.autowording-backups`, plus optional read-back verification of title, caption, keywords, category and rights values after every write.
- Batch save: selected items with titles written sequentially, progress bar, per-file errors.
- Agency CSV export/import: Generic, Adobe Stock, Shutterstock, iStock, Dreamstime and Alamy layouts with RFC 4180 quoting; an export preflight lists each affected file once with all of its missing fields, too few keywords, or unmapped Adobe/Shutterstock category issues before writing, and shows a dedicated warning when category cells will be blank. Import matches rows by file name, fills title/caption/keywords/editorial/category, maps Adobe numeric codes and Shutterstock category names back to app labels, preserves unknown free-text values, normalizes keywords, leaves empty cells untouched, and reports unmatched rows. Import reports that a folder must be open first.
- Safe overwrites: before writing, the app compares the file's size and modification time with the values captured when its metadata was read; a file changed by another program is skipped with a clear error. **Restore backup** (Metadata tab) or **More → Restore backups** copies the original back and re-reads its metadata.
- Unsaved-edit journal: dirty inspector fields are mirrored to `edit-journal.json` every ~400 ms and re-applied when the folder is reopened, so closing or crashing does not lose manual title/caption/keyword edits. Entries disappear once the file is saved or restored.
- Viewer mode: large active image, previous/next, 86×64 filmstrip, and **Back to browser**. There is no position counter, dimensions overlay or error overlay in the GPUI Viewer; metadata and errors are shown in the inspector.
- Folder browser is a lazy/cached **This PC** tree of mounted drives and their folders, plus Quick access shortcuts; it auto-expands to the current folder. The GPUI sidebar has no breadcrumb bar and no parent-navigation shortcut.
- Four providers: Ollama, LM Studio, OpenAI, Anthropic.
- Endpoint model discovery: **Load** in Settings fetches models from Ollama `/api/tags`, LM Studio and OpenAI `/v1/models`, and Anthropic `/v1/models` with provider authentication; results appear as selectable model buttons.
- ExifTool auto-detection, manual path override, version display, re-check, Unicode-safe argfile execution.
- Config persistence: `%APPDATA%\AutoWording\config\config.toml`, including appearance, density, inspector collapse state, marketplace profile, title/caption ranges, keyword-target, backup, verification, rights metadata preferences, and per-path field locks.
- Navigation: Browser/Viewer, Open/Add/Generate/Save, theme, density, and the **More** actions are mouse-driven in the GPUI view. **Ctrl+B** toggles the inspector rail from either state. During an AI batch, **Ctrl+P** pauses, **Ctrl+Shift+P** resumes, and **Ctrl+Shift+C** cancels after the current image.
- 216 tests: video extension classification, state video items, XMP/QuickTime Keys write and clear arguments, grouped video parsing and Keys fallback, prompt/parser, range-aware prompt and validation, configurable keyword acceptance percentage, export preflight, per-image rights and undo, batch keyword cleanup, case-insensitive duplicate removal, persisted locks, target-aware normalization, singular/plural de-duplication, title/caption, keyword, non-English-title/caption retry, identical-output detection and escalated retry, English-only keyword filtering, direct save after every generation, category prompting and parsing, rights tag arguments and read-back verification, and malformed-reply retry, field-lock merging, safe-write backup/verification, all providers and model-list endpoints via mock HTTP, metadata tags including GPS/date, real ExifTool round-trips including PNG and XMP-only orientation, selection math, system folder tree and breadcrumbs, FIFO retry ordering and retry limits, queue-card retry requeueing, batch pause/resume/cancel shortcuts, Batch-tab keyword undo, cache lifecycle, config, Windows theme detection, EXIF and XMP orientation parsing and rotation, GPUI tab switching, animated inspector collapse and minimum-window layout, and Windows inspector snapshots.

---

## 4. Requirements

### 4.1 Runtime

- Windows 10 or 11.
- [ExifTool](https://exiftool.org/) installed and:
  - on `PATH` as `exiftool`, or
  - selected in **Settings → ExifTool**, or
  - placed in one of the auto-detected locations in §12.2.
- A vision model:
  - Ollama with `llava`, `minicpm-v`, `qwen2-vl`, etc.; or
  - LM Studio with a vision model and local server enabled; or
  - OpenAI API key; or
  - Anthropic API key.

### 4.2 Build

- Rust stable ≥ 1.92.
- A working linker:
  - MSVC toolchain + Visual Studio Build Tools + Windows SDK, or
  - GNU toolchain `stable-x86_64-pc-windows-gnu` plus the complete MinGW installation at `%ProgramData%\mingw64\mingw64\bin` (this machine's setup). The partial linker tools in `.tools\gnu-bin` are a fallback only; the complete MinGW `gcc` supplies `cc1` from `libexec` and `windres` from `bin`.

`reqwest` uses `native-tls`/Schannel, so no C compiler is required. Do not switch back to `rustls` without a C toolchain; `ring` previously failed here.

---

## 5. Build, run, test

### 5.1 Canonical helper

```bat
build.bat test
build.bat build --release
build.bat run --release
```

`build.bat` prepends the complete MinGW `bin` directory (`MINGW64_BIN`, default `%ProgramData%\mingw64\mingw64\bin`) ahead of `%~dp0.tools\gnu-bin`, exports `CC`/`AR`/`WINDRES`, and calls:

```bat
cargo +stable-x86_64-pc-windows-gnu %*
```

Keeping MinGW first lets its `gcc` resolve `cc1.exe` in `libexec` and `windres.exe` in `bin`; putting `.tools\gnu-bin` first would use its partial GCC 14.2 driver and fail with `cannot execute 'cc1'`. It exists because the MSVC toolchain on this machine lacks the Windows SDK (`kernel32.lib` missing) and Git's `link.exe` shadows the MSVC linker.

### 5.2 Verified Bash command

```bash
MINGW_BIN="${MINGW64_BIN:-/c/ProgramData/mingw64/mingw64/bin}"
SELF="$USERPROFILE/.rustup/toolchains/stable-x86_64-pc-windows-gnu/lib/rustlib/x86_64-pc-windows-gnu/lib/self-contained"
export PATH="$MINGW_BIN:$PWD/.tools/gnu-bin:$SELF:$PATH"
export CC="$MINGW_BIN/gcc.exe"
export AR="$MINGW_BIN/ar.exe"
export WINDRES="$MINGW_BIN/windres.exe"
cargo +stable-x86_64-pc-windows-gnu test
cargo +stable-x86_64-pc-windows-gnu build --release
```

The complete `MINGW_BIN` directory supplies `cc1` through GCC's `libexec` layout and `windres`; the `SELF` directory supplies the GNU linker's self-contained runtime.

### 5.3 Normal MSVC build

With Visual Studio Build Tools and the Windows SDK:

```bat
cargo test
cargo run --release
```

### 5.4 Ignored paths

- `target/` — Cargo output.
- `.tools/` — partial linker fallback (`gcc.exe`, `ld.exe`, `dlltool.exe`, `as.exe`); the complete MinGW installation supplies `cc1` and `windres`.
- `.freebuff/` — local agent metadata.
- `%APPDATA%\AutoWording\config\config.toml` — user settings and API keys.
- `dist/` — packaged release executables written by `release.ps1` or the release workflow.

`[profile.release] opt-level = 2, strip = true, lto = "thin"` — the release build drops symbols and runs thin link-time optimization. Measured on the same source: rebuilding after touching one file takes about **53 s** and gives a **27.4 MB** exe, against about **3 min 55 s** and **21.4 MB** with full `lto = true`, so thin LTO trades roughly 6 MB of exe size for a build roughly four times faster. There is an embedded application icon, but no manifest, installer, code signing, or auto-update. `build.rs` compiles `app.rc` with `embed-resource`, which embeds `assets/autowording.ico` on Windows builds.

### 5.5 Local release

```bat
release.bat
release.bat 0.2.0
release.bat 0.2.0 -DryRun
```

`release.ps1` builds through `build.bat` (GNU toolchain), copies `target\release\autowording.exe` to `dist\AutoWording-v<version>-windows-x64.exe`, and publishes a GitHub release tagged `v<version>` whose asset is that exe.

- Version defaults to `Cargo.toml`; the tag is `v` plus that version.
- Release notes default to the commit subjects since the previous tag; pass `-NotesFile notes.md` to supply curated Markdown.
- Publishing uses the REST API when `GH_TOKEN` or `GITHUB_TOKEN` is set, otherwise the authenticated GitHub CLI (`gh`). Nothing is published without one of those.
- The current commit must already be pushed: the tag is created at `HEAD`, and the script refuses to run when `origin/<branch>` is missing or behind.
- `-NoBuild` reuses the existing exe; `-DryRun` prints the tag, asset, and notes without publishing.
- Uncommitted changes are only a warning (the binary includes them) but the tag and notes describe the pushed commit.
- The exe is not committed; `dist/` is gitignored.

### 5.6 Automated GitHub release

`.github/workflows/release.yml` runs on a `v*` tag push and on manual dispatch (`windows-latest`, MSVC toolchain). A tag push publishes; a manual run only builds and uploads a workflow artifact:

1. Reads `version` from `Cargo.toml` and fails when a pushed tag is not `v<version>`.
2. Runs `cargo test --locked -- --skip inspector_tabs_match_snapshots`; the DirectX baseline comparison stays a local check because it needs a visible desktop.
3. Builds `cargo build --release --locked`.
4. Packages `dist\AutoWording-v<version>-windows-x64.exe`, uploads it as a workflow artifact, and — for a tag push — creates or updates the GitHub release with commit-subject notes. A re-run of the tag workflow replaces the existing asset with `gh release upload --clobber`.

To publish a release, bump `version` in `Cargo.toml`, commit, push, then push the matching tag:

```bat
git tag v0.2.1
git push origin v0.2.1
```

The workflow uses the automatic `GITHUB_TOKEN` with `contents: write`; no repository secret is needed. `release.bat` remains the local GNU-toolchain alternative.

---

## 6. End-user workflow

### 6.1 Open and browse

1. Launch `autowording.exe`. The left panel shows **This PC**, mounted drives, and Quick access folders. The last folder from config is reopened and revealed automatically if it still exists.
2. Expand a drive or folder, or use a Quick access shortcut. Clicking a folder loads its direct images and MP4/MOV videos into the grid.
3. **Open** in the top bar opens the native folder picker.
4. **Refresh** in the sidebar or browser header rescans drives, refreshes the current folder, and clears cached tree children while preserving expansion.
5. **Add** appends arbitrary supported images and MP4/MOV videos chosen with the native file picker.
6. The sidebar has no breadcrumb bar and no parent-folder shortcut; use the tree to move elsewhere.

Below the header, the status buttons narrow the grid (All, Pending, Queued, Working, Generated, Saved, Failed, Modified) and the search field matches file name, title, caption, category and keywords. The **Sort** button cycles name, status, keyword count and modified time; **Ascending/Descending** flips its direction. **Clear** resets both. The header reports `N visible`. Filtered-out images stay in the folder and keep their statuses; selection operations act on the visible subset.

### 6.2 Select

- **Click** a card: select only that image and make it the active image.
- **Ctrl+Click**: toggle that card without clearing others.
- **Shift+Click**: select the contiguous range from the anchor to the target.
- **Ctrl+Shift+Click**: add that range to the existing selection.
- **Select all** (grid action): selects every image in the current filtered view.
- There is no Clear-selection button; plain-clicking another card replaces the selection.
- Use the top-bar **Viewer** control to open the active image.

The **active** image is the primary focus, shown in Viewer and used by the single inspector. It can be part of a larger selection.

### 6.3 Single-image editing

With exactly one selected image or video, the Metadata tab shows:

- File name, status pill and dimensions (or `Dimensions pending`).
- Title and Keywords fields on one row.
- Caption textarea.
- **Lock title**, **Lock caption**, **Lock keywords**, **Lock category** checkboxes; at narrow rail widths they shorten to **Title**, **Caption**, **Keywords**, **Category** under a **LOCKS** label. Locked fields survive AI results, so regenerating keeps a manually typed category.
- **Editorial** switch (off means commercial), and a **Category** field for the short stock label written to IPTC/XMP.
- **Rights overrides**: Creator, Copyright, Credit line and Usage terms. Existing values read from the file populate these fields as per-image overrides; blank fields fall back to the Settings values, and non-empty values apply to that image only and are verified after writing. Each field shows a **File** badge for a non-empty per-image override or a **Settings** badge when blank, so the source of the value being written is visible at a glance.
- **Undo last save** restores the metadata captured before the most recent successful write (five writes of history per image), marks the image Modified, and leaves the file unchanged until Save.
- **Generate** (each successful result is written automatically) and **Save to EXIF** for manual edits. For MP4/MOV, Generate reports that AI video generation is unavailable; Save writes the video's XMP and QuickTime Keys.
- **Restore backup** restores the first pre-write copy.
- **File details**: capture date, GPS location, and the last error message when present.
- The **Force regenerate** switch lives in the browser header (and the Batch tab), not the inspector.

Editing a `Generated` or `Saved` image sets `dirty` and changes the badge to **Modified**. Editing a `Pending`/`Error` image leaves the status but makes it saveable once a title exists.

### 6.4 Batch selection editing

With 2+ images selected, the Batch tab shows:

- `N images selected` and `Apply shared metadata to N selected image(s).`
- Live queue card while a batch runs: AI queue or Save queue, `Working`/`Paused`/`Writing`/`Cancelling`, percent, a progress bar, `done of total complete`, ETA, current file, up to three up-next filenames plus a remaining count, and Done/Waiting/Failed counts, with **Pause**/**Resume**, **Retry all failed** while failures remain, and **Cancel** during AI runs (Cancel stops after the current image). Pressing **Cancel** immediately disables it, swaps in a **Cancelling…** spinner, clears Pause, and disables the other queue controls until the current request finishes. The same controls are available as **Ctrl+P**, **Ctrl+Shift+P**, and **Ctrl+Shift+C**.
- Title field + **Apply title** (overwrites every selected title).
- Caption field + **Apply caption**.
- Keywords field + **Append keywords** (dedupe, cap 50) or **Replace keywords**.
- Keyword cleanup: **Find** / **Replace** with **Replace term**, **Remove duplicates**, **Normalize**, **Sort A–Z** and **Clear keywords** across the selection. Remove duplicates drops case-insensitive repeats while preserving the first spelling and order; Normalize dedupes; Clear requests explicit tag removal on save. **Append keywords** and **Replace keywords** share the same single-step undo as the cleanup actions. Each operation that changes at least one file replaces the previous one-step snapshot, and **Undo cleanup** restores every selected file's keywords and clear-keyword flag from before that operation. An operation that changes nothing leaves the existing snapshot untouched. While the Batch tab is active, **Ctrl+Z** runs the same undo.
- **Generate selected**, **Save selected**, **Retry failed** (selected failures), and the live queue card's **Retry all failed** (all failures in the running batch).
- **Force regenerate complete images** switch.
- Shared values are not aggregated: the tab is an explicit apply editor, not a live common/mixed view. With several images selected, the Metadata tab shows an `N images selected` banner and an **Open batch editor** button.

Batch fields are not a live merged editor; they are explicit apply operations. A blank title/caption cannot be applied. Keywords use the same normalizer as the AI parser (alphabetic words, dedupe, max 50).

### 6.5 Save and auto-save

- AI generation auto-saves every result as soon as field locks have been applied, before the next image is requested. Writes stay sequential through ExifTool, use `-overwrite_original`, and go through the same backup/verification settings.
- With **Back up originals before writing** enabled (default), each image is copied to `.autowording-backups` before its first write. Repeated saves keep the first backup.
- Before every write (manual or automatic), the app compares the file's current size and modification time with the values captured when its metadata was read. If the file changed in another program, that save is refused with `changed on disk since it was loaded`; refresh and review before writing again. Files with no captured baseline are written normally.
- **Restore backup** (Metadata tab) or **More → Restore backups** copies the first backup back over the selected files, clears their in-memory metadata, re-reads it from disk, and reports how many were restored. Restore is confirmed first because it replaces the current file contents.
- Unsaved manual edits are continuously journaled to `%APPDATA%\AutoWording\config\edit-journal.json` (debounced to 400 ms) and flushed synchronously when the window closes. Journal writes replace the file atomically, so an interrupted write leaves the previous journal intact. On launch, journal entries whose file still exists are applied to freshly loaded items, shown as **Modified**, and kept until the image is saved or restored. A corrupt journal is ignored, and entries for deleted files are dropped.
- Closing the window or choosing **More → Exit AutoWording** with dirty images cancels the close and shows an **Unsaved changes** warning with **Save all and exit**, **Exit without saving** and **Keep editing**. **Save all and exit** writes every dirty image, then closes; if any file fails or has no title, the warning returns instead of closing. Exiting still flushes the journal, so the fields survive the relaunch, but the files keep their previous metadata until Save is used.
- With **Verify metadata after writing** enabled (default), the file is read back after writing and the save fails if title, caption, keywords, category, or any configured creator/copyright/credit/usage-terms value does not match.
- If ExifTool is unavailable, generation still completes, but the result stays `Generated` and the app reports that it was not saved.
- Single: **Save to EXIF** (Metadata tab) or **Save** in the top bar writes the active image or video; for MP4/MOV the label is unchanged but the write targets XMP and QuickTime Keys. An empty keyword list normally leaves file keywords untouched; **Clear keywords** or emptying the inspector field requests explicit `-IPTC:Keywords=` / `-XMP-dc:Subject=` removals on images and `-XMP-dc:Subject=` / `-Keys:Keywords=` removals on video. **Undo last save** restores the metadata from before the last successful write so it can be reviewed and saved again.
- Batch: **Save selected** (Batch tab) or **Save** in the top bar writes every selected image with a non-empty title.
- Items without a title are skipped; the toast reports the skipped count.
- Success sets `Saved` and clears `dirty`; failure keeps the previous status and shows `Save failed: …`.
- Manual edits made after generation are not auto-saved; use Save before closing.

### 6.6 Viewer

- Switch with the top-bar **Viewer** control.
- Large image with letterboxing, or a `VIDEO / Preview unavailable` placeholder; the file name is shown in the Viewer header. Videos are not played.
- **Previous**, **Next**, and **Back to browser** are the navigation controls.
- Horizontal filmstrip of 86×64 px thumbnails in a 92 px strip; click a thumbnail to change the active image.
- No position counter, dimensions or error overlay in the GPUI Viewer; use the Metadata tab for dimensions, capture date, GPS and errors.

### 6.7 Choose a provider and model

1. Open the **Settings** tab in the inspector.
2. Choose the provider. Its endpoint is prefilled with the provider default and remains editable.
3. Press **Load** beside the model field to fetch the endpoint's model IDs. Discovery is on demand; it does not run automatically when the tab opens.
4. Click one of the returned model buttons, or type a model ID into the field.
5. If discovery fails, the model stays editable; a manually entered ID is still usable.
6. Cloud providers require an API key before their model list can be fetched. Local providers need no key.

The list is sorted case-insensitively, deduplicated, and limited to model IDs (`name` for Ollama, `id` for the OpenAI-compatible and Anthropic APIs). Model discovery does not call the chat/generation endpoint.

### 6.8 Marketplace profiles

**Settings → Marketplace output** selects the stock agency the output is written for. Choosing a profile adopts that agency's default output limits; the keyword target and the title/caption range fields are then capped by the profile, so the prompt and generated output follow the same limits.

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

Caps mirror each agency's published upload limits (keyword counts and title/description character limits) converted to the app's units; titles are measured in words at roughly 12 characters per word. The app's global bounds still apply, and the metadata writer never emits more than 50 keywords. Shutterstock requires at least five title words, so that profile's default title range starts at 5. Switching profiles never rewrites existing metadata: it changes the defaults and caps for the next generation and the Settings fields. The choice is stored as `marketplace` in `config.toml`.

---

### 6.9 CSV export and import

- **More → Export current selection** writes exactly the selected images and videos — an empty selection reports "Select one or more images to export" instead of exporting the grid — in one of six layouts: **Generic CSV** (`Filename,Title,Caption,Keywords,Editorial`), **Adobe Stock** (`Filename,Title,Keywords,Category,Releases`), **Shutterstock** (`Filename,Description,Keywords,Categories,Illustration,Mature content,Editorial`), **iStock / Getty** (`File Name,Title,Description,Keywords`, the four-column qHero-compatible form), **Dreamstime** (`Filename,Title,Description,Keywords`) or **Alamy** (`Filename,Caption,Tags,Supertags`). Fields are RFC 4180 quoted, lines are CRLF terminated, and the default file name is `autowording-<agency>.csv`. Exports replace the chosen file atomically, preserving an existing CSV if the write fails. A preflight runs first and lists each affected file once with all of its missing-title/caption, keyword-count, or unmapped Adobe/Shutterstock category issues; **Export anyway** writes the file, **Cancel** returns to the grid.
- The Adobe Stock `Category` and Shutterstock `Categories` columns are filled from the app category because those agencies accept only their own codes and names: Adobe gets its numeric code and Shutterstock its official category name. Examples: `Nature`/`Landscapes` → `11` / `Nature`, `Travel` → `21` / `Parks/Outdoor` (Shutterstock has no Travel category), `People` → `13` / `People`, `Food` → `7` / `Food and drink`, `Business` → `3` / `Business/Finance`, `Technology` → `19` / `Technology`, `Architecture` → `2` / `Buildings/Landmarks`, `Animals` → `1` / `Animals/Wildlife`. Synonyms such as `Wildlife`, `Interiors`, `Flowers`, `Church`, `Portrait` and `Abstract` resolve too; a blank or unrecognised category leaves the cell empty rather than sending invalid text. The file's IPTC/XMP category still carries the original free-text label. Releases, iStock's created date/country/brief code and Alamy supertags are likewise left blank rather than guessed; agency templates change, so verify headers against the current template before a bulk import. Alamy normally matches by Image Ref after upload, so its filename-based layout is for the template contributors send to support. Selected MP4/MOV files export the same filename/title/caption/keywords/category/editorial fields as images.
- **More → Import CSV** reports that a folder must be open when none is selected, then matches rows to the current folder by base file name, case-insensitively. It accepts the exported layouts plus `Headline`, `Caption`, `Tags`/`Subject`, `File Name` and `Editorial Status` headers. Non-empty cells overwrite title/caption/keywords/editorial/category; empty cells keep the existing value. Adobe's numeric `Category` column is converted back to an app category (for example `11` → `Nature`), and Shutterstock's `Categories` names are matched case-insensitively (for example `Animals/Wildlife` → `Animals`). Unknown codes, names, and free-text category values are kept unchanged. Keywords go through the normal normalizer (alphabetic single words, deduped, capped at 50).
- Imported rows become **Modified** (or **Generated** when title, caption and keywords are all present) and are selected; the toast reports unmatched rows. Import never writes files — review and use **Save** to persist. Export uses the in-memory values, so unsaved edits are included.

## 7. UI reference

### 7.1 Layout

```
┌──────────────────────────────────────────────────────────────────────────────────────┐
│ AW  AutoWording   Open  Add  Generate  Save    Browser│Viewer  Light  Hide panel  More │
├────────────────────┬──────────────────────────────────────────────┬────────────────────┤
│ LIBRARY   Refresh  │ BROWSER  24 visible   [search]       Refresh  │ INSPECTOR          │
│  Quick access      │ 2026_Stock                                   │ [Metadata|Batch|Settings] │
│  This PC           │ All Pending Queued … Modified                │                    │
│   ▸ C:\            │ Select all      Sort: Name  Ascending  Force │ Title              │
│     ▸ Photos       │ ┌─────┬─────┬─────┬─────┐                     │ Caption            │
│       ▸ 2026_Stock │ │card │card │card │card │                     │ Keywords           │
│                    │ └─────┴─────┴─────┴─────┘                     │ File details       │
├────────────────────┴──────────────────────────────────────────────┴────────────────────┤
│ 24 images · 3 selected      Ollama · llava      ExifTool ready     Comfortable        │
└──────────────────────────────────────────────────────────────────────────────────────┘
```

- Window: 1440×920 initial bounds, opened maximized; minimum 1024×680.
- Top bar: 58 px tall, 16 px horizontal padding, 8 px gaps; a 32 px rounded brand tile with `AW`, the AutoWording wordmark over `AI METADATA STUDIO`, and a 24 px divider before **Open**, **Add**, **Generate**, **Save**. The right side holds **Browser**, **Viewer**, **Light/Dark**, **Hide panel/Show panel**, **More**.
- Sidebar: 220 px fixed; `LIBRARY` eyebrow with **Refresh**; 28 px folder rows indented 13 px per depth, with chevrons for expandable folders; the current folder uses the accent fill and shows its direct-image count.
- Center: browser header with `BROWSER`, `N visible`, the folder name, a 220 px search field, and optional **Clear** / **Refresh**; then the status filter row and the action row (**Select all**, **Sort**, **Ascending/Descending**, **Force regenerate**). Cards wrap in a vertically scrolling grid with 16 px padding and 12 px gaps.
- Inspector: 320–356 px rail, sized as 30% of the window width clamped to that range. The 1024×680 minimum gets 320 px, leaving ~484 px for the grid; larger windows reach the 356 px maximum. It has an `INSPECTOR` eyebrow and a three-way segmented control. Each tab button is 34 px high and equal width; the active tab uses the background fill and bold text. **Hide panel** springs the rail width to zero while clipping and fading its content; **Show panel** springs it back. The collapsed state is saved in config and restored on the next launch.
- Status bar: 30 px. The left side shows counts, the toast, or batch progress (`Generating 3 / 12`, failed count, a 110×5 px bar, ETA). The right side shows the provider/model line, a **Queue** button during a batch, `ExifTool ready`/`ExifTool unavailable`, and the **Comfortable/Compact** toggle.
- Grid card: 168 px wide, 168×126 px thumbnail area, 12 px radius.
- Viewer filmstrip: 86×64 px thumbnails in a 92 px strip.
- Light and dark palettes cover panels, cards, inputs, dialogs, badges and the preview letterbox. GPUI Kit's default font stack is used; the app does not load Segoe UI itself.

### 7.2 Top bar and More actions

There is no menu bar. The top bar and the **More** modal hold every command:

| Control | Action |
| --- | --- |
| **Open** | Native folder picker; loads the folder's direct images |
| **Add** | Native file picker; appends supported images and MP4/MOV videos |
| **Generate** | Runs the sequential AI batch on the selection; disabled without a selection or while a batch runs |
| **Save** | Writes selected images that have a title; disabled without a selection or while a batch runs |
| **Browser / Viewer** | Switches workspace mode |
| **Light / Dark** | Toggles the persisted light/dark preference |
| **Hide panel / Show panel** (`Ctrl+B`) | Collapses or restores the inspector rail with a spring animation |
| **More** | Modal with Import CSV, Restore backups, About, the six agency CSV export rows, Exit AutoWording and Close |

The **Settings** tab holds provider, endpoint, model **Load**, API key, context, global instructions, marketplace, keyword/title/caption ranges, ExifTool path and **Re-check ExifTool**, Follow Windows/Light/Dark, Comfortable/Compact, backup and verification switches, and the config path. Browser/Viewer and density changes keep the selection.

### 7.3 Statuses

| Status | Card label | Meaning |
| --- | --- | --- |
| `Pending` | Pending (gray) | Title, caption or keywords missing, or fewer than the configured percentage of the keyword target |
| `Queued` | In queue (blue) | Waiting for the current image to finish |
| `Processing` | Processing (blue) | AI request in flight |
| `Generated` | Generated (violet) | Parsed AI result awaiting its immediate auto-save |
| `Saved` | Saved (green) | Written to disk |
| `Error` | Failed (red) | Generation/save error; the message appears in File details |
| `Pending`/`Generated`/`Saved` + dirty | Modified (amber) | Edited before or after generation/save |

`ImageItem::is_pending()` is true for `Pending | Error`; `Queued` is not retried by that helper. `retry_failed` explicitly filters `Error`. Files that already contain a title, caption and the configured acceptance percentage of the keyword target (rounded up) are loaded as `Saved` (`ImageItem::is_complete(target, percent)`), so a folder of finished images does not show a wall of Pending badges; files below that floor stay `Pending` so AI can complete them. Batch generation skips every saved item or item with complete metadata (`ImageItem::is_complete(target, percent)`), including generated-but-unwritten results, so finished work is not sent to the model again; editing a complete image does not make it a generation target either. The browser-header **Force regenerate** switch (also in the Batch tab) bypasses the skip for the next run.

### 7.4 Keyboard shortcuts

`Ctrl+B` toggles the inspector rail. During an AI batch, `Ctrl+P` pauses, `Ctrl+Shift+P` resumes, and `Ctrl+Shift+C` cancels after the current image. These are registered in `ui::init_keybindings` as the `ToggleInspector`, `PauseBatch`, `ResumeBatch`, and `CancelBatch` actions for the `AutoWording` key context; the inspector action follows the same persisted config path as the top-bar button. The batch shortcuts are no-ops without a running AI batch or while the save queue is active, matching the queue-card controls. The legacy `Shortcuts` struct in `src/app.rs` remains unused. Inside a focused text field the normal Windows editing keys apply (`Ctrl+A` selects that field's text, `Enter` submits single-line inputs). Dialogs close with their buttons; `Esc` is not wired.

### 7.5 Grid card details

- 168 px card with a 168×126 px thumbnail area, 12 px radius, 1 px border and secondary fill.
- Selected cards use the primary color as their border; there is no separate focus ring.
- Thumbnails use cover fit; failed decodes show `Preview unavailable`, pending ones `Loading preview`. Video cards show `VIDEO / Preview unavailable` without attempting a decode.
- Below the thumbnail: a bold truncated file name; a left-aligned status label colored by state; a right-aligned `N tags` count.
- Status text is `Pending` / `In queue` / `Processing` / `Generated` / `Saved` / `Failed`, `Reading metadata` while a metadata read is pending, and amber `Modified` for dirty editable items.
- No checkbox, no dimensions overlay, no card tooltip.
- Hover does not load metadata; selection does.

### 7.6 System folder browser

- **Quick access** lists Desktop, Downloads, Documents, and Pictures when those system folders exist.
- **This PC** lists mounted drive roots (`C:\`, `D:\`, …); drives are lazy and can be expanded in place.
- Directory children load only when a folder is expanded and are cached per directory.
- Directories sort case-insensitively; `$RECYCLE.BIN` and `System Volume Information` are omitted.
- Opening a folder reveals and expands its whole ancestor chain inside the system tree instead of replacing the tree root.
- Current folder is highlighted and shows its direct-image count.
- Opening a folder clears selection, preview, and metadata-request cache.
- **Refresh** in the sidebar or browser header rescans mounted drives, clears tree caches, and preserves expansion state.
- There is no breadcrumb row and no `Alt+Up` handling; the tree is the only folder navigation.
- The grid always shows only the current folder's direct images and MP4/MOV videos, not media from its subfolders.

---

## 8. Architecture

### 8.1 Runtime/thread model

```
┌────────────────────────────────────────────────────────────┐
│ GPUI main thread (AutoWordingView::render)                 │
│  - paints top bar, sidebar, grid/Viewer, inspector, modals │
│  - owns App: items, folder_images, Selection, images      │
│  - _pump every 120 ms: poll_events + sync_journal + notify │
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
- GPUI renders on the main thread; decoded images are converted into GPUI render images there.
- Background tasks send events through an unbounded channel; no repaint call is needed, because `_pump` calls `cx.notify()`.
- `_pump` is a GPUI task (`cx.spawn_in`) that sleeps 120 ms, then calls `App::poll_events`, `sync_journal`, removes the window when `close_now` is set, and calls `cx.notify()`.
- Config changes set `cfg_dirty`; `poll_events` saves after 500 ms.
- `sync_journal` runs after event polling, at most every 400 ms: it copies dirty items into `edit-journal.json`, keeps entries for other folders, drops entries for missing files, and removes entries for saved or restored images. `flush_journal` bypasses the debounce for window close, and a close request with dirty items is canceled while the warning is shown.

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
  ├─ skip items with complete metadata (counted for the toast),
  │  unless Force regenerate is enabled
  ├─ dedupe paths, snapshot existing title/caption/keywords/GPS/date, set items to Queued
  ├─ BatchProgress { total: unique images, done: 0, failed: 0, saving: false }
  └─ spawn async coordinator with a FIFO AiQueue
       └─ until the queue is empty:
            ├─ check Cancel; wait while Pause is active
            ├─ send AiStarted → UI sets Processing
            ├─ spawn_blocking(prepare_image)
            ├─ ai::generate(AiRequest { existing, context, ... })
            ├─ on failure with retries left: push the job to the back,
            │  send AiRetryScheduled, and continue with the next queued job
            ├─ on success or final failure: count the completed image
            ├─ send AiDone; successful results enter the auto-save queue
            └─ start the next image only after this one finishes
       └─ after the queue drains: return untouched Queued items to Pending
          and send BatchFinished
```

`AiDone` replaces the existing Title/Caption/Keywords/Category with the generated values plus Editorial unless that field is locked, sets `Generated` and `dirty`, queues the path for an immediate save, and increments `batch.done`. `AiRetryScheduled` puts the item back in `Queued` with a `Retrying n/2` message; it does not increment progress. `batch.done` therefore counts unique images only, never retry attempts. `pump_auto_save` writes a successful image before the next request is made, keeping one ExifTool write in flight at a time; a failed write leaves the item `Generated` with a save error and the queue continues. A generation is marked `Error` only after both retries are exhausted. **Retry all failed** in the queue card sends those terminal errors back to the worker through a bounded retry channel; they are appended to the FIFO queue, reset to `Queued`, and counted out of the failed/done totals so the progress bar reflects the reopened work. Cancel stops before the next queued image, but every already-generated result is still written. `BatchFinished` clears the AI batch slot, flushes any remaining queued save, reports how many complete files were skipped, and reports when ExifTool is unavailable so nothing was written.

### 8.4 Save flow

```
save_paths(paths)
  ├─ used by Save Selected; per-image auto-saves reuse the same
  │  backup/write/verify block from their own one-at-a-time queue
  ├─ reject if batch active / no ExifTool
  ├─ items with has_metadata() (title non-empty)
  ├─ BatchProgress { total, done: 0, failed: 0, saving: true }
  └─ spawn sequential loop
       ├─ optional backup_original → .autowording-backups/<name>
       ├─ spawn_blocking(ExifTool::write_with_rights)
       ├─ optional ExifTool::read + verify_written + verify_rights
       ├─ send SaveDone (Saved, dirty=false, or error message)
       └─ send BatchSaved; clear batch slot
```

Saves are deliberately sequential to avoid launching many ExifTool processes over the same folder. Automatic per-image saving reuses the same backup/write/verify block, but drains its own queue instead of taking the batch slot, so generation can continue while the previous file is written.

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
            ├─ populate title/caption/keywords only if all empty AND status Pending
            └─ mark Saved when title, caption and keywords are all present
```

`meta_requested` prevents duplicate reads. It is cleared on folder refresh/open and when ExifTool becomes ready.

### 8.6 Event reference (`src/state.rs::WorkerEvent`)

| Event | Payload | UI handling |
| --- | --- | --- |
| `Thumbnail` | path, decoded image | create a render image and cache it |
| `Preview` | path, decoded image | replace the Viewer preview image |
| `ImageFailed` | path, error | mark cache failure + item message |
| `AiStarted` | path | `Queued → Processing`; record the file for status-bar progress |
| `SaveStarted` | path | record the file being written for status-bar progress |
| `AiDone` | path, `Result<GeneratedMetadata, String>` | fill fields or set `Error`; progress++ |
| `BatchFinished` | ok, failed | clear AI batch, flush queued saves; toast with the skipped count |
| `SaveDone` | path, `Result<(), String>` | `Saved`/message; progress++ |
| `BatchSaved` | ok, failed | clear batch; toast |
| `MetaLoaded` | path, `ExistingMetadata` | fill empty pending item |
| `ModelsLoaded` | provider, endpoint, `Result<Vec<String>, String>` | populate the Settings model buttons; ignore stale endpoint/provider results |
| `ExifToolReady` | `Option<ExifTool>`, status | store tool; retry metadata for the selection |

### 8.7 Core data structures

```rust
enum Status {
    Pending, Queued, Processing, Generated, Saved, Error,
}

struct ImageItem {
    path: PathBuf,
    media: MediaKind,       // Image | Video
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

struct BatchProgress { total: usize, done: usize, failed: usize, saving: bool }

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
release.ps1                    release build + GitHub release publishing
release.bat                    cmd wrapper for release.ps1
README.md                      this document

src/main.rs                    Windows subsystem attr, GPUI application/window setup (1440×920, min 1024×680)
src/app.rs                     controller: folders, selection, workers, batches, config/theme/density
src/state.rs                   ImageItem/Status/Selection, folder scan, decode, events
src/config.rs                  Provider, ProviderSettings, Config load/save/defaults, density, marketplace
src/theme.rs                   Windows app-theme detection (registry)
src/persist.rs                 atomic state-file writes used by config and the journal
build.rs                       embeds the Windows icon resource during the build
app.rc                         Windows resource script for the application icon
assets/autowording.ico         multi-resolution 16–256 px AW application icon
src/cache/mod.rs               cache module root + ThumbnailCache re-export
src/cache/thumbnails.rs        thumbnail cache, in-flight/failure sets, lifecycle test

src/ui/mod.rs                  module root: AutoWordingView re-export, UiAction
src/ui/app_view.rs             the whole GPUI view: top bar, sidebar, grid, Viewer, inspector tabs, modals, UI tests
src/ui/tree.rs                 lazy system folder tree: drives, Quick access, reveal, expansion tests

tests/snapshots/<scale>/*.png inspector baselines for the screenshot test (1.5 and 2.0 display scales)

src/ai/mod.rs                  AiRequest, prepare_image, provider dispatch, endpoint, snippet
src/ai/prompt.rs               exact prompt, tolerant parser, keyword normalizer, validation
src/ai/ollama.rs               Ollama payload/chat
src/ai/lmstudio.rs             LM Studio payload, endpoint normalization, chat
src/ai/models.rs               provider model-list discovery, endpoint normalization, parsing
src/ai/openai.rs               OpenAI-compatible payload/chat
src/ai/anthropic.rs            Anthropic Messages payload/chat
src/ai/tests.rs                hand-written TCP mock HTTP server tests for all providers and model lists

src/metadata/mod.rs            ExifTool detection, argfile protocol, image/video read/write, round-trips
src/metadata/tags.rs           image and video read tag lists, group-qualified JSON parser, read tests
src/metadata/writer.rs         Metadata type, image/video tag-writing argument builders, writer tests
src/orientation.rs             JPEG EXIF/XMP orientation parsing and pixel rotation
src/csv.rs                     Agency CSV layouts, RFC 4180 export/import parsing, filename index
src/journal.rs                 Unsaved-edit journal: JSON load/save, stale-entry pruning, tests
```

`src/ui/browser.rs`, `src/ui/metadata_panel.rs` and `src/ui/viewer.rs` are leftover egui modules; `src/ui/mod.rs` does not declare them, so they are not compiled and are not part of the running app. Do not treat them as the current UI.

Dependency roles:

| Crate | Role |
| --- | --- |
| `gpui-kit` 0.9.0 | GPUI window, widgets, theme, DirectX 11 rendering |
| `gpui-pre` 0.3.11 (dev, `test-support`) | Headless GPUI test harness (`#[gpui_kit::test]`, simulated clicks) |
| `gpui-pre-platform` 0.3.11 (dev, `test-support`) | Off-screen platform for the inspector screenshot test |
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
| `image` 0.25 (`jpeg`, `png`) | Decode, resize, re-encode; PNG only for test snapshots |

---

## 10. Selection engine

`Selection` is intentionally independent of GPUI so it can be tested without a window.

### 10.1 Rules

- `selected`: path set.
- `anchor`: starting point for Shift ranges; normally the last plain/Ctrl-click.
- `active`: image shown in Viewer and used by the single inspector; always selected after a click.
- `primary(ordered)`: returns active if it is still selected; otherwise the first selected path in grid order.

### 10.2 Behavior

| Gesture | Result |
| --- | --- |
| Plain click | Clear, select target, active = anchor = target |
| Ctrl+Click selected | Deselect target; active moves to first remaining selected path |
| Ctrl+Click unselected | Add target; active = anchor = target |
| Shift+Click | Clear (unless Ctrl held), select range anchor↔target, active = target; anchor stays |
| Ctrl+Shift+Click | Add range to current selection, active = target |
| Programmatic toggle | Toggle target without clearing; active = target when selecting, first remaining when deselecting. GPUI cards have no checkbox. |
| Select all button | Select all grid paths; anchor = first; active preserved if still valid, else first |
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
6. Parse with `prompt::parse_response_allow_empty_keywords`. The parsed title is reduced to plain words first: punctuation is deleted or turned into a word break, and a token carrying a digit is dropped whole (`3rd` disappears rather than leaving `rd`), so a stray full stop, quote or year never reaches the title tag. Letters outside English are kept so the non-English check below still rejects them. Keyword normalization removes non-English keywords, exact duplicates and singular/plural pairs (keeping the singular), then trims to the target. A keyword containing any non-ASCII character is dropped whole rather than truncated.
7. If the request fails or the reply cannot be parsed, retry up to three times (300, 600, 900 ms apart). A parse failure reuses a repair prompt that restates the five labelled lines and includes the previous reply.
8. If the title/caption are outside their configured ranges, contain non-English letters, or the parsed reply has fewer than the configured percentage of the target keywords (rounded up), make one corrective request at **temperature 0.7** (the first pass uses 0.5). The retry prompt shows the previous title/caption verbatim, forbids reusing that wording, and names an exact target length (the range midpoint) so the model cannot land on the same boundary again.
9. Each repaired field is accepted on its own: a reply whose title fits the configured range and uses English letters only replaces the first title, and the caption is checked separately (its editorial flag travels with it), so fixing one field is never discarded because the other one overran. Retry keywords are always merged into the first list, preserving order and dropping duplicates/singular-plural pairs before capping at the target. If a constraint is still unmet, make one escalated request at **temperature 0.95** with the photographer's extra instructions removed, asking the model to write an `Observation:` line first and build the four fields from it; a retry that returned the same title and caption is named explicitly. Its valid title and valid caption each replace their failed field, and its keywords fill the short list.
10. If the title and caption are now valid and only the keyword count is short, make one keywords-only request: the accepted title, caption, category and current list are repeated and the model must add the missing number of single-word terms on a single `Keywords:` line. It is sent **without the image** under its own short system instruction, so it costs little and cannot rewrite the text that already passed. Those terms are merged into the accepted list with the same duplicate and singular/plural rules, so an otherwise correct reply is no longer discarded over the last few keywords. If the metadata still fails after the top-up, return an error so the image can be retried.
11. Return the metadata unchanged: there is no stock-policy gating or human review state. The caller marks the image `Generated` and writes it immediately, so every successful generation reaches disk.

No streaming or cancellation. Excess keywords are trimmed before the result reaches the UI.

### 11.2 Image preparation

- `image::open` (JPEG only in this build); MP4/MOV files are filtered out before preparation.
- Downscale longest side to **1600 px** with `FilterType::Triangle` if larger.
- Always convert to RGB8 and re-encode JPEG quality **85**.
- Base64 standard encoding, no `data:` prefix.
- Strips all metadata before transmission.
- EXIF orientation, then XMP orientation when EXIF has no tag, is applied before downscaling, so portrait photos are analyzed upright.

### 11.3 Exact system prompt

`SYSTEM_TEMPLATE` in `src/ai/prompt.rs`; `{DATE}` is replaced with `Month D, YYYY` (English month, no leading zero), `{KEYWORD_TARGET}` with the configured 1–50 target, `{TITLE_MIN_WORDS}`/`{TITLE_MAX_WORDS}` with the configured title range, and `{CAPTION_MIN_CHARS}`/`{CAPTION_MAX_CHARS}` with the configured caption range, and `{TITLE_AIM_WORDS}`/`{CAPTION_AIM_CHARS}` with the midpoint of each range, so the first pass is given one exact length instead of only a range. If `global_prompt` is non-empty:

```
\n\nAdditional instructions from the photographer (follow these too):\n<trimmed global_prompt>
```

Full template:

```text
Role & Task:
You are an expert photography analyst specializing in metadata tag generation, keywording, and stock image description. Analyze the provided image and generate accurate titles, captions, and keywords following strict guidelines.

Accuracy and safety rules:
1. Ground every claim in the image or supplied data.
   - Name a landmark, building, city, country, species, brand, person, or event only when it is clearly identifiable from visible evidence or supplied GPS/context.
   - Never guess a location, date, event, name, brand, species, or release status. If uncertain, use a generic description and leave the detail out.
   - Do not include trademarks, brand names, product names, real people's names, personal information, URLs, email addresses, phone numbers, or hashtags in commercial metadata.
2. Commercial vs. Editorial Status:
   - Mark EDITORIAL when the image contains a recognizable face, logo/trademark, private property, minor, or newsworthy/sensitive event, unless the supplied release status says commercial use is cleared.
   - For documentary editorial content, open the caption with the place and the supplied date, then a colon and one factual sentence:
     City, Country - Month Day, Year: factual sentence about the subject.
   - Fill that prefix from this image, not from the template. A correctly filled example is: Hong Kong, China - {DATE}: A tram passes a crowded market street. The place must be the supplied GPS/context location or one clearly identifiable in the image.
   - Never output City, Country, Month Day, or Year as literal placeholders, and do not reuse the example's place or sentence unless the image shows it. If no place is known, start with the factual sentence and leave the prefix out.
   - Use the supplied editorial date only. It must be the date the image was taken. If the supplied date is not a capture date, do not invent one; leave the date out.
   - The editorial location must come from supplied GPS/context. If no location is supplied, do not invent one; describe the scene without it.

Editorial date supplied for this request: {DATE}

Hard rules for all five lines (one violation invalidates the reply):
- English only: every word in plain English letters, ASCII only, no accents or non-English characters (write "cafe", not "café").
- Plain text only: no markdown, no quotation marks or backticks around values, no emoji, no hashtags, and no symbols such as &, /, |, @, #, %, +, =, (, ), [, ], {, }.
- No URLs, email addresses, phone numbers, prices, or calls to action, and no promotional wording.
- One value per labelled line, with nothing before, between, or after the five lines.

Output Formatting Constraints:

1. Title:
   - Length: Between {TITLE_MIN_WORDS} and {TITLE_MAX_WORDS} words; aim for exactly {TITLE_AIM_WORDS} words. Count the words before replying.
   - Content: A complete descriptive sentence about the primary subject, not a comma-separated keyword list.
   - Language: English letters only. No accented or non-English letters.
   - Characters: letters and single spaces only. No punctuation at all: no periods, commas, hyphens, apostrophes, colons, digits, or other special characters. Do not end with a period.
   - Do not use promotional language, URLs, contact details, or all caps.

2. Caption:
   - Length: Strict limit of {CAPTION_MIN_CHARS} to {CAPTION_MAX_CHARS} characters including spaces; aim for about {CAPTION_AIM_CHARS} characters. Count every character before replying and stay inside the limit even when the scene is complex.
   - Content: Factual description of the scene (or the editorial structure above).
   - Language: English letters only. No accented or non-English letters.
   - Characters: letters, digits, spaces, commas, and periods only. The editorial location/date prefix may use the single hyphen and the colon shown above; use no other symbols, quotes, hashtags, or emoji.
   - Do not use promotional language, URLs, contact details, hashtags, or keyword stuffing.

3. Keywords:
   - Quantity: Exactly {KEYWORD_TARGET} keywords. Count them before replying; a shorter list is an error even when the scene is simple.
   - Format: Single words only (no multi-word phrases/keyphrases).
   - Separator: Comma-separated.
   - Rules: English letters only (A-Z, a-z). No numbers, punctuation, special characters, or accented letters.
   - Relevance: Only include terms visibly supported by the image. Remove generic filler, title repeats, brands, real-person names, and speculative concepts. When obvious subjects run out, continue with single words for visible colours, materials, textures, light, weather, time of day, season, framing, and mood instead of stock-agency words or filler.
   - Duplicates: Do not include a plural when its singular form is also present. Keep the singular (for example, keep "apple" and omit "apples"). Never repeat a keyword or vary its spelling just to fill the count.

4. Category:
   - One short stock category that best fits the visible subject, in Title Case. Examples: `Nature`, `Travel`, `People`, `Food`, `Business`, `Technology`, `Architecture`, `Animals`.
   - Use one or two plain words, letters and single spaces only. Do not use a sentence, a comma-separated list, a keyword list, or punctuation.

Final check before replying (silently fix any line that fails; do not explain):
- Title has {TITLE_MIN_WORDS}-{TITLE_MAX_WORDS} words, English letters and spaces only.
- Caption has {CAPTION_MIN_CHARS}-{CAPTION_MAX_CHARS} characters including spaces, English only.
- Keywords contain exactly {KEYWORD_TARGET} unique single English words, with no plural paired with its singular.
- No line contains markdown, quotation marks, emoji, URLs, or non-English letters.

Required Output Schema:
Title: [{TITLE_MIN_WORDS}-{TITLE_MAX_WORDS} words]
Editorial Status: [Commercial / Editorial]
Caption: [{CAPTION_MIN_CHARS}-{CAPTION_MAX_CHARS} characters]
Keywords: [word1, word2, word3, ... word{KEYWORD_TARGET}]
Category: [one short stock category]

Reply with the five labelled lines above and nothing else. Do not use markdown.
```

The batch date is computed once as a fallback. For each image, a valid EXIF capture date overrides it for the editorial prompt; if the capture date is missing, the batch date is used instead.
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

Use these details as context and return fresh title, caption, keywords, and category.
```

Context is trimmed; blank = absent:

```text
Situational context supplied by the photographer (treat it as ground truth):
<context>

Analyze the attached image and return the five labelled fields exactly as specified.
```

Without metadata or context, the prompt is only the final analyze line.

If normalization leaves fewer than the configured percentage of the target keywords (rounded up; for example 40 of 50 at 80%), a second request appends:

```text
The previous reply contained only N usable keywords; at least MINIMUM (the configured percentage of TARGET, rounded up) are required. Return the five labelled fields again, keeping the same title and caption. Provide at least MINIMUM unique single-word keywords. Do not include both a singular and its plural; keep the singular. Previous keywords: ...
```

### 11.5 Provider wire formats

| Provider | Default base URL | Final endpoint | Auth | Image field | Sampling | Response |
| --- | --- | --- | --- | --- | --- | --- |
| Ollama | `http://localhost:11434` | `/api/chat` (base `/api` → `/api/chat`) | none | `messages[1].images = [base64]` | `stream:false`, temp 0.5 | `message.content`, fallback `response`; checks `error` |
| LM Studio | `http://localhost:1234/v1` | full `/chat/completions`, or `/v1` + `/chat/completions`, or bare + `/v1/chat/completions` | none | OpenAI `image_url` data URI | temp 0.5, max 1400 | `choices[0].message.content` |
| OpenAI | `https://api.openai.com/v1` | `/chat/completions` | `Authorization: Bearer` | `image_url` data URI | temp 0.5, max 1400 | content string, or text parts joined by `\n` |
| Anthropic | `https://api.anthropic.com/v1` | `/messages` | `x-api-key`, `anthropic-version: 2023-06-01` | base64 image block, `image/jpeg` | temp 0.5, max 1400; system top-level | text blocks joined by `\n` |

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

Shared by AI parsing and the Batch tab's append/replace actions:

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

`Settings → Load` fetches the list of model IDs from the selected provider's endpoint, rather than requiring the ID to be typed from memory.

| Provider | Model-list request | Authentication | Parsed field |
| --- | --- | --- | --- |
| Ollama | `GET /api/tags` | none | `models[].name` (fallback `model`) |
| LM Studio | `GET /v1/models` | none | `data[].id` |
| OpenAI | `GET /v1/models` | `Authorization: Bearer <key>` | `data[].id` |
| Anthropic | `GET /v1/models` | `x-api-key`, `anthropic-version: 2023-06-01` | `data[].id` |

Endpoint normalization accepts the provider defaults plus common user variants: Ollama base `/api` or `/api/chat` maps to `/api/tags`; LM Studio bare host, `/v1`, or `/v1/chat/completions` maps to `/v1/models`; OpenAI/Anthropic full chat/messages URLs map back to `/v1/models`.

The request has a 20-second timeout. HTTP/JSON errors include the provider and a body snippet. Cloud providers reject discovery without a key before sending a request. Results are sorted case-insensitively and deduplicated. A configured real model missing from the response is inserted into the model buttons so it can still be selected. The retired LM Studio `local-model` placeholder is never inserted; when LM Studio has no model configured, the first endpoint model is selected. Discovery is asynchronous through `WorkerEvent::ModelsLoaded`; stale responses from an old provider/endpoint are ignored. The `Load` button retries after an error.

---

## 12. Metadata subsystem

### 12.1 Tag mapping

Image tags:

| Field | Tags written |
| --- | --- |
| Title | `XMP-dc:Title`, `IPTC:ObjectName`, `EXIF:XPTitle` |
| Caption | `EXIF:ImageDescription`, `IPTC:Caption-Abstract`, `XMP-dc:Description` |
| Keywords | repeating `IPTC:Keywords`, repeating `XMP-dc:Subject` |
| Category | `IPTC:Category`, `XMP-photoshop:Category` |
| Creator | `XMP-dc:Creator`, `IPTC:By-line` |
| Copyright | `XMP-dc:Rights`, `IPTC:CopyrightNotice` |
| Credit line | `XMP-photoshop:Credit`, `IPTC:Credit` |
| Rights usage terms | `XMP-xmpRights:UsageTerms` |

- Caption empty + title present → only `EXIF:ImageDescription=title`.
- Title empty → no title tags.
- Both empty → no descriptive tags.
- Keyword list empty normally → no keyword arguments, so existing file keywords remain. **Clear keywords** (Batch tab) or emptying the inspector field requests explicit `-IPTC:Keywords=` / `-XMP-dc:Subject=` removals on save.
- `writer::write_args` caps keywords at 50.
- Empty category → no category arguments, so an existing file category remains.
- Rights values come from Settings, overridden by any non-blank per-image Creator/Copyright/Credit line/Usage terms value. Existing file values are loaded into those overrides when metadata is read, so saving preserves file-specific rights; blank fields continue to use Settings. The model never supplies rights. Each blank setting is skipped, so it never erases an existing value on disk; every save writes all non-blank rights fields.

Video tags (`MP4`/`MOV`):

| Field | Tags written |
| --- | --- |
| Title | `XMP-dc:Title`, `Keys:Title` |
| Description | `XMP-dc:Description`, `Keys:Description` (caption, falling back to title) |
| Keywords | repeating `XMP-dc:Subject`, one comma-separated `Keys:Keywords` |
| Category | `XMP-photoshop:Category` |
| Creator | `XMP-dc:Creator`, `Keys:Author` |
| Copyright | `XMP-dc:Rights`, `Keys:Copyright` |
| Credit line | `XMP-photoshop:Credit` |
| Rights usage terms | `XMP-xmpRights:UsageTerms` |

- MP4/MOV never receives IPTC or EXIF tags, and `video_write_args` caps keywords at 50.
- Empty video keywords normally leave both lists untouched; an explicit clear writes `-XMP-dc:Subject=` and `-Keys:Keywords=`.
- XMP wins over QuickTime Keys when reading the same field, and the two keyword lists are merged case-insensitively.

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

A candidate is accepted only if `exiftool -ver` succeeds and prints a non-empty version. Detection runs in `spawn_blocking`; the version appears in the status bar and in **Settings → ExifTool**.

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

Image base flags, always:

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

A non-empty category is written to both category tags:

```text
-IPTC:Category=Nature
-XMP-photoshop:Category=Nature
```

The four image rights settings are appended to every image write, each line only when its setting is non-blank:

```text
-XMP-dc:Creator=Jane Doe
-IPTC:By-line=Jane Doe
-XMP-dc:Rights=© 2026 Jane Doe
-IPTC:CopyrightNotice=© 2026 Jane Doe
-XMP-photoshop:Credit=Jane Doe
-IPTC:Credit=Jane Doe
-XMP-xmpRights:UsageTerms=Commercial use only
```

ExifTool treats `=` like `+=` if the same tag is also set with `+=`/`-=` in the same command. Pure repeated `=` replaces the list. `overwriting_metadata_replaces_previous_keywords` verifies this against real ExifTool.

MP4/MOV use `-P -overwrite_original -m` (no IPTC charset flag) and write both metadata families:

```text
-XMP-dc:Title=Harbour at Dawn
-Keys:Title=Harbour at Dawn
-XMP-dc:Description=Waves break against the stone pier.
-Keys:Description=Waves break against the stone pier.
-XMP-photoshop:Category=Nature
-XMP-dc:Subject=harbour
-XMP-dc:Subject=dawn
-Keys:Keywords=harbour, dawn
-XMP-dc:Creator=Jane Doe
-Keys:Author=Jane Doe
-XMP-dc:Rights=Copyright 2026 Jane Doe
-Keys:Copyright=Copyright 2026 Jane Doe
-XMP-photoshop:Credit=Jane Doe
-XMP-xmpRights:UsageTerms=Commercial use
```

### 12.5 Read arguments and precedence

```text
-j -charset iptc=utf8
-Title -ObjectName -Description -Caption-Abstract -ImageDescription
-XPTitle -IPTC:Keywords -Subject -IPTC:Category
-XMP-dc:Creator -XMP-dc:Rights -XMP-photoshop:Credit -XMP-xmpRights:UsageTerms
-ImageSize -Orientation#
<path>
```

`parse_read_json`:

- Requires a JSON array with at least one object.
- Title: `Title` → `XPTitle` → `ObjectName`; empty/whitespace falls through.
- Caption: `Description` → `Caption-Abstract` → `ImageDescription`.
- Keywords: merge `Keywords` then `Subject`; array or comma-separated string; trim/drop empty; case-insensitive dedupe.
- Category: `Category` (IPTC) → `Category` (`XMP-photoshop`, used for PNG), trimmed; empty values are ignored.
- Rights: `Creator` (`XMP-dc`), `Rights` (`XMP-dc`), `Credit` (`XMP-photoshop`) and `UsageTerms` (`XMP-xmpRights`), trimmed; used by `verify_rights` after a save.
- `ImageSize` only lowercase `WIDTHxHEIGHT`; uppercase `X` rejected.
- `Orientation` (numeric via `#`) swaps the reported width and height for values 5–8, because those orientations rotate the displayed image; unknown or missing values keep `ImageSize` order.
- When ExifTool reports no usable Orientation, `ExifTool::read` scans the JPEG's APP1 segments and uses XMP `tiff:Orientation` / `exif:Orientation` (element or `rdf:Description` attribute form) as the fallback, swapping dimensions for 5–8. EXIF orientation always wins.
- Invalid JSON/empty array/object instead of array = error.
- Wrong-type values are ignored.
- Output decoded with `from_utf8_lossy` before JSON parsing.

MP4/MOV reads add `-G1` and request group-qualified tags: `XMP-dc:Title`, `Keys:Title`, `ItemList:Title`, `UserData:Title`; the same title/description fallbacks; `XMP-dc:Subject` and `Keys:Keywords`; category, creator/author, rights/copyright, credit and usage terms; `ImageSize` and `CreateDate` (plus `Keys:CreationDate`). `parse_video_read_json` accepts `Composite:ImageSize` (`WIDTHxHEIGHT`), prefers XMP over Keys for single-value fields, merges both keyword lists without case-insensitive duplicates, and takes the first capture date from `CreateDate`, `QuickTime:CreateDate`, `XMP-xmp:CreateDate` or `Keys:CreationDate`. Batch reads split images and videos into separate ExifTool calls so each group uses its own parser.

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

`ProjectDirs::config_dir()` appends `config`. The Settings tab shows the exact path. Fallbacks: executable directory, then relative `config.toml`. The unsaved-edit journal is `edit-journal.json` in the same directory.

`Config::load_with_warning`:

- Reads if present.
- On parse error, copies the original to `config.toml.corrupt-<timestamp>`, shows a startup warning, and uses **all defaults**.
- On a read error other than a missing file, shows a startup warning and uses defaults.
- On missing file, uses defaults and immediately saves.
- Sets `path` (serde-skipped, never written).

`Config::save` creates parent directories, writes a flushed temporary file, then renames it over `config.toml`, so an interrupted save leaves the previous settings intact. UI debounces 500 ms.

### 13.2 Schema and defaults

```rust
enum Provider { Ollama, LmStudio, OpenAi, Anthropic } // serde: snake_case
struct ProviderSettings { base_url, model, api_key }
struct Config {
    provider,
    ollama, lmstudio, openai, anthropic: ProviderSettings,
    global_prompt,
    creator,            // XMP-dc:Creator / IPTC:By-line, written on every save
    copyright,          // XMP-dc:Rights / IPTC:CopyrightNotice
    credit_line,        // XMP-photoshop:Credit / IPTC:Credit
    rights_usage_terms, // XMP-xmpRights:UsageTerms
    exiftool_path,
    last_folder,        // reopened at startup
    theme,              // system | light | dark
    density,            // comfortable | compact
    inspector_collapsed, // rail collapsed state restored at startup
    marketplace,        // general | adobe_stock | shutterstock | istock | dreamstime | alamy | depositphotos | pond5
    keyword_target,     // desired keywords per image, clamped 1..=50 at use
    keyword_accept_percent, // accepted share of the target, clamped 1..=100 at use
    title_min_words,    // title minimum words, clamped 1..=20 at use
    title_max_words,    // title maximum words, clamped 1..=20 at use
    caption_min_chars,  // caption minimum characters, clamped 10..=1000 at use
    caption_max_chars,  // caption maximum characters, clamped 10..=1000 at use
    backup_originals,   // copy into .autowording-backups before the first write
    verify_writes,      // read back and compare after each write
    field_locks,        // map of lowercased absolute path -> per-field locks
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
| `creator` / `copyright` / `credit_line` / `rights_usage_terms` | empty (blank settings write nothing) |
| `exiftool_path` | empty (auto-detect) |
| `last_folder` | empty |
| `theme` | `system` (follows the Windows app theme) |
| `density` | `comfortable` |
| `inspector_collapsed` | `false` |
| `marketplace` | `general` |
| `keyword_target` | `50` |
| `keyword_accept_percent` | `80` |
| `title_min_words` / `title_max_words` | `3` / `5` |
| `caption_min_chars` / `caption_max_chars` | `50` / `180` |
| `backup_originals` | `true` |
| `verify_writes` | `true` |
| `field_locks` | empty (no paths locked) |

TOML provider values: `ollama`, `lm_studio`, `open_ai`, `anthropic`; theme values: `system`, `light`, `dark`; density values: `comfortable`, `compact`. Unknown values fail the whole parse and fall back to defaults. `#[serde(default)]` fills missing fields/sections.

```toml
provider = "open_ai"
global_prompt = "Prefer British spelling and avoid brand names."
creator = "Jane Doe"
copyright = "© 2026 Jane Doe"
credit_line = "Jane Doe"
rights_usage_terms = "Commercial use only"
exiftool_path = ""
last_folder = "C:\\Photos\\2026_Stock"
theme = "system"
density = "comfortable"
inspector_collapsed = false
marketplace = "general"
keyword_target = 50
keyword_accept_percent = 80
title_min_words = 3
title_max_words = 5
caption_min_chars = 50
caption_max_chars = 180
backup_originals = true
verify_writes = true

[field_locks.'c:\photos\2026_stock\sample.jpg']
title = true
caption = false
keywords = true
category = false

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

The marketplace value clamps the three output limits at use time (prompt, generated output, and the Settings fields). A hand-edited `keyword_target = 50` with `marketplace = "adobe_stock"` is used as 49; an Alamy caption range with `caption_max_chars = 400` is used as 150. `Config::apply_marketplace` writes the profile's defaults into those fields when the user switches profiles.

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
MINGW_BIN="${MINGW64_BIN:-/c/ProgramData/mingw64/mingw64/bin}"
SELF="$USERPROFILE/.rustup/toolchains/stable-x86_64-pc-windows-gnu/lib/rustlib/x86_64-pc-windows-gnu/lib/self-contained"
export PATH="$MINGW_BIN:$PWD/.tools/gnu-bin:$SELF:$PATH"
export CC="$MINGW_BIN/gcc.exe"
export AR="$MINGW_BIN/ar.exe"
export WINDRES="$MINGW_BIN/windres.exe"
cargo +stable-x86_64-pc-windows-gnu test
```

Expected: **215 passed; 0 failed; 0 ignored**.

List: `cargo test -- --list`.

### 14.2 Inventory

| Module | Count | Coverage |
| --- | --- | --- |
| `ai::prompt::tests` | 38 | markdown/CRLF/numbered/full-width labels, duplicate labels, multiline values, editorial precedence, title cleanup, keyword normalization/Unicode/dedupe/cap, case-insensitive duplicate removal, singular/plural de-duplication, range-aware prompt/validation/trimming, retry prompt with verbatim previous reply and exact targets, escalation prompt, identical-text comparison, unparsable-reply repair prompt, validation boundaries, exact editorial caption, existing metadata/context prompt |
| `ai::tests` | 36 | mock HTTP sequence server for all providers; model-list endpoints, auth headers, parsing and no-key validation; payload/header/endpoint assertions; malformed JSON; HTTP errors; missing fields; image resize/encoding/rejection; snippet truncation; short-keyword, title/caption, malformed-reply, and escalated retries; retry temperatures |
| `config::tests` | 17 | defaults, serde names, accessors, save/load round-trip incl. `last_folder`/`theme`/`density`/`keyword_target`/ranges/rights, density serde/clamping, target and range clamping, marketplace profile caps/defaults and profile switching, legacy LM Studio placeholder cleanup, invalid TOML, path stability |
| `metadata::tags::tests` | 15 | image read precedence/fallbacks/merge/dedupe/invalid payloads/string keywords, GPS and capture-date parsing, rights field reading, orientation-aware dimensions, XMP fallback precedence; group-qualified video parsing, XMP/Keys keyword merge, `Composite:ImageSize` dimensions, Keys-only fallback |
| `metadata::writer::tests` | 11 | all image tags, title fallback, empty metadata, no `+=`, Unicode, 50-keyword safety cap, rights tag arguments and blank-field skipping; video XMP/QuickTime Keys arguments and video keyword clearing |
| `metadata::tests` | 16 | real ExifTool round-trip and write verification, rights round-trip and verification, XMP-only orientation fixture, safe backup preservation, external-change guard, overwrite semantics, 50 keywords, Unicode values, Unicode file names, missing files, explicit/directory detection, argfile formatting |
| `state::tests` | 38 | image/video extension classification, MP4/MOV folder scans, video item defaults, PNG/TIFF decode, decode resize/orientation/corrupt, item conversion, field-lock merging, progress incl. failed count, selection math, Ctrl/Shift/toggle/retain/clear, keyword append, singular/plural append, browser text/status filtering (filename/title/caption/keywords) and name/status/keyword sorting |
| `orientation::tests` | 9 | EXIF APP1 parsing (little- and big-endian TIFF), XMP element/attribute and `exif:` prefix fallbacks, EXIF precedence, invalid XMP values, rotation/transpose dimensions, clockwise pixel placement |
| `csv::tests` | 11 | generic round-trip with quotes/commas/newlines, agency headers and columns, Adobe/Shutterstock category mapping and reverse import mapping, unmapped/blank cells, grouped per-filename preflight issues, Shutterstock/Adobe/generic header mapping, editorial flags, missing Filename and blank rows, case-insensitive filename index |
| `journal::tests` | 4 | dirty-edit round-trip, empty journal removes the file, corrupt journal ignored, entries for deleted files dropped |
| `app::tests` | 5 | a journal entry restores title/caption/keywords/editorial as a dirty Modified item; every accepted generation, commercial or editorial, is queued for auto-save while failures are not; field locks persist per path and restore at startup; retry-all-failed requeues batch errors; one-step batch keyword undo restores the whole selection and clear-keyword state after normalize, clear, append and replace |
| `cache::thumbnails::tests` | 1 | request lifecycle blocks duplicates/failures |
| `theme::tests` | 1 | Windows theme registry parsing |
| `ui::tree::tests` | 4 | sorted children/expansion/depth; refresh preserves expansion and rereads children; reveal expands only the ancestor chain; breadcrumb path segments |
| `ui::app_view` | 8 | GPUI tab switching, animated inspector collapse, 1024×680 minimum layout, fixture metadata statuses, batch pause state, batch pause/resume/cancel shortcuts, Batch-tab Ctrl+Z keyword undo, inspector header and full metadata-rail snapshot comparison |
| `persist` | 2 | atomic replacement without a leftover temp file, temp-file cleanup on failure |

ExifTool integration tests skip with `exiftool not available; skipping test` if detection fails. This keeps the suite runnable without ExifTool, but those tests only prove metadata correctness when ExifTool is installed. On this machine they run.

Provider tests make no real API calls. `src/ai/tests.rs` has a TCP HTTP server on `127.0.0.1:0` that can return a sequence of responses; it reads each request, asserts method/path/headers/body, and returns canned JSON. The keyword and title/caption retry tests use two responses; the escalation tests use three (initial, corrective, escalated), and the give-up test consumes all four (the first request plus three parse retries).

Temporary artifacts: `%TEMP%\autowording-tests`, `autowording-state-tests`, `autowording-config-tests`, `autowording-tree-tests`, `autowording-persist-tests`.

The Windows-only inspector snapshot test renders two real, unfocused DirectX windows at the top-left of the desktop: 1280×800 (rail at 356 px) and 1024×680 (rail at 320 px). It seeds a selected Generated image so the Metadata tab renders its full content, then captures Metadata, Batch and Settings headers in both windows, normalizing each crop to 2× its rail width (712×184 and 640×184). It also captures the entire Metadata rail from the top bar to the status bar at both widths (712×1424 and 640×1184), covering the compact lock row at 320 px. Comparisons use `tests/snapshots/<scale>/inspector-<tab>.png`, `inspector-min-<tab>.png`, and the full `inspector-metadata-full.png` / `inspector-min-metadata-full.png` baselines (up to 0.05% pixels may differ). The test selects the directory for the actual Windows display scale; baselines exist for 1.5 and 2.0. Other scales fail with an instruction to generate that scale's set, so a monitor change cannot silently compare against the wrong rasterization. Regenerate the baselines after an intentional header or metadata-layout change:

```bat
set UPDATE_INSPECTOR_SNAPSHOTS=1
build.bat test inspector_tabs_match_snapshots
```

A failing run writes `target/<snapshot-name>-actual.png`.

### 14.3 Not tested

- GPUI interaction beyond the eight `ui::app_view` tests: dialogs and modals, native file pickers, scrolling, drag-and-drop (not implemented), and keyboard shortcuts other than Ctrl+B, Ctrl+Z on the Batch tab and the batch pause/resume/cancel shortcuts.
- Visual layout beyond the inspector header and full metadata-rail snapshots; the full window, grid, Viewer and Settings content are not snapshotted.
- Live Ollama/LM Studio/OpenAI/Anthropic calls.
- Windows native file dialogs (`rfd`).
- Real folder volumes, permission errors, junctions/symlink loops.
- ExifTool behavior on corrupt JPEG, PNG or TIFF files.
- Interactive pause/cancel behavior and process interruption; cancellation is implemented between images but is not UI-automated.
- Memory behavior with thousands of images.
- Non-Windows builds.

---

## 15. Invariants and gotchas

1. **Repeated `=` replaces; any `+=` makes `=` behave like append.** Do not mix.
2. **UTF-8 argfile is mandatory on Windows.** Direct `argv` loses accents/CJK.
3. **`-overwrite_original` is irreversible.** Backups are opt-out via `backup_originals`. **Undo last save** restores the in-memory snapshot from before the last write (five per image) and requires Save; it never restores file bytes.
4. **Metadata only loads into empty `Pending` items.** Prevents clobbering generation/user edits. A file that already has a title, caption and keywords is promoted to `Saved` instead of staying `Pending`.
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
16. **Successful generation auto-saves immediately, per image.** Each result is written before the next image starts, one ExifTool process at a time; without ExifTool, results remain `Generated` and are not written.
17. **LM Studio endpoint normalization** accepts host, `/v1`, or full `/chat/completions`.
18. **Ollama base ending `/api`** becomes `/api/chat`.
19. **Provider failures retry automatically.** Each image gets one request plus up to three retries (300/600/900 ms) for transport errors and unparsable replies; the range/keyword corrective pass is a separate final request. Items that still fail become `Error` and can be retried via **Retry failed**.
20. **The model sees a re-encoded, orientation-corrected JPEG**, quality 85, max 1600 px; fine detail may be lost.
21. **EXIF orientation is applied**, with XMP `tiff:Orientation` / `exif:Orientation` as the fallback when EXIF has no tag, when decoding thumbnails, previews and AI input; dimensions shown in the UI are post-rotation.
22. **Keywords are single words by design**; multi-word concepts split.
23. **Keyword dedupe preserves first spelling**, case-insensitive.
24. **Unicode keywords pass validation** (`is_alphabetic`), despite the prompt asking for alphabetic characters.
25. **Editorial toggle does not rewrite the caption.**
26. **Folder scans are non-recursive for the grid.** `add_paths` uses a recursive collector for folder paths, but the GPUI Add button only picks files.
27. **The left tree is rooted at the system, not the opened folder.** Mounted drives and Quick access stay available; opening a folder only reveals its ancestor chain.
28. **`folder_root` is separate from `current_folder`**: `folder_root` is the opened folder's volume root; the current folder is the selection.
29. **Refresh keeps existing selections and leaves an empty selection empty.**
30. **`meta_requested` is cleared on folder open/refresh/ExifTool-ready**, causing re-reads.
31. **The app is not code-signed**; SmartScreen may warn.
32. **No installer, auto-update, single-instance enforcement, or tray icon.**
33. **Manual edits made after generation are not auto-saved**; generation auto-saves once.
34. **Saves refuse externally modified files.** The fingerprint is file size plus modification time, captured on `MetaLoaded` and refreshed after a successful save; a mismatch blocks the write instead of clobbering outside edits.
35. **Restore uses the first backup only.** `.autowording-backups/<name>` is copied back and metadata is re-read; there is no per-save version history.
36. **The edit journal only carries unsaved fields.** It stores title, caption, keywords, editorial, the four rights overrides and the keyword-clear flag for dirty items — not field locks, GPS/date, selection or batch queue. Field locks persist separately in `config.toml` by path. Restored entries appear as `Generated` + dirty (**Modified**) and still require Save.
37. **Closing with unsaved edits warns first.** A close request is canceled while the warning is open; **Save all and exit** saves every dirty image first, **Exit without saving** flushes the journal and closes, and **Keep editing** keeps the app open. **More → Exit AutoWording** uses the same path. If the save leaves failures or untitled images, the warning comes back rather than closing. With no dirty items the window closes normally.

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

Edit `Selection` in `state.rs`, then update `state::tests` for plain/Ctrl/Shift/Ctrl+Shift/toggle/retain. The GPUI card click handler only converts Ctrl/Shift into `SelectModifiers`; keep that translation thin.

### 16.5 Change batch processing

`generate_paths` builds a FIFO `AiQueue` and runs one async loop: send `AiStarted`, prepare the image, call `ai::generate`, then send `AiDone` or `AiRetryScheduled`. A failure is pushed to the back with its retry count; after two retries (`AI_MAX_RETRIES`) the final `AiDone` records the failure. Initial jobs and retries share the same queue, so retries never preempt waiting work. Images waiting for their turn stay `Queued`. Preserve that distinction if refactoring.

### 16.6 Add UI tests

The view already has a GPUI harness. `#[gpui_kit::test]` provides a `TestAppContext`; `cx.add_window_view` builds `AutoWordingView::new_with_core(App::from_config(...))`; `cx.debug_bounds("selector")` reads a `debug_selector`; `cx.simulate_click`, `cx.simulate_keystrokes` and `cx.simulate_resize` drive the real layout. Eight `ui::app_view` tests cover tab switching, the Ctrl+B collapse shortcut, batch pause/resume/cancel shortcuts, the Batch-tab Ctrl+Z keyword undo, minimum size, metadata statuses, pause state and snapshots.

For a new visual check, add a `debug_selector` to the region, capture it with `window.render_to_image` (Windows/DirectX), normalize the crop and compare it with a PNG in `tests/snapshots`. Regenerate with `UPDATE_INSPECTOR_SNAPSHOTS=1` and the test name. Do not claim coverage for dialogs, pickers or shortcuts that are not exercised.

---

## 17. Troubleshooting

| Symptom | Likely cause / fix |
| --- | --- |
| `ExifTool not found` | Install it, add to `PATH`, or set the path in **Settings → ExifTool**, then **Re-check ExifTool**. |
| Settings reset unexpectedly | Check `config.toml.corrupt-<timestamp>` beside the config; the app preserves an unparseable file there before using defaults. |
| Save disabled | No ExifTool, no selection, no title, or a batch is running. |
| `cannot execute 'cc1'` or missing `windres` | `.tools\gnu-bin` is ahead of the complete MinGW compiler. Run `build.bat`, or put `%ProgramData%\mingw64\mingw64\bin` first and set `CC`/`WINDRES` there. |
| Grid empty | The folder has no direct supported files (`.jpg`, `.jpeg`, `.jpe`, `.jfif`, `.png`, `.tif`, `.tiff`, `.mp4`, `.mov`); use **Add** or select a different folder. |
| Tree does not show a subfolder | It starts with `.` or is one of the skipped names; refresh after creating it. |
| `Ctrl+A` types in a field instead of selecting | Expected: `Ctrl+A` selects the focused field's text. There is no global Select all shortcut; use the **Select all** button. |
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
- Images are JPEG/PNG/TIFF; video is MP4/MOV metadata only. No HEIC/RAW/MPG, video thumbnails/playback or AI video generation.
- Writes into the original file; no XMP sidecars.
- No per-image cancel; Pause/Cancel stop between images.
- No automatic retries/backoff/rate-limit handling/cost accounting.
- Undo is limited to restoring the first pre-write backup; no per-save history, metadata diff, or raw ExifTool output viewer.
- No keyword clearing when the list is empty.
- No keyword translation/synonym expansion/per-market limits.
- Queue/selection are in memory only; config and the unsaved-edit journal persist.
- Folder grid does not show subfolder images recursively.
- No installer, signing, auto-update, tray icon, single-instance enforcement.
- No pull-request CI; the tagged release workflow runs the suite before publishing, except the snapshot comparison.
- No license file. Add one before redistribution (default is all rights reserved).

---

## 19. Glossary

| Term | Meaning |
| --- | --- |
| **ACDSee layout** | Browser mode with folder tree + thumbnail grid + inspector; Viewer mode for a single large image. |
| **Microstock** | Low-cost royalty-free/editorial stock marketplaces. |
| **Editorial** | Newsworthy/documentary content; recognizable people/events usually need releases for commercial use. |
| **Commercial** | Advertising-usable content. |
| **IPTC** | News metadata: `Keywords`, `ObjectName`, `Caption-Abstract`. |
| **XMP** | Adobe XML metadata: `dc:title`, `dc:description`, `dc:subject`. |
| **EXIF** | Camera/image metadata: `ImageDescription`, `XPTitle`. |
| **ExifTool** | Phil Harvey's Perl metadata CLI used here. |
| **Argfile** | UTF-8 file passed with `-@`, one ExifTool argument per line. |
| **Shutterstock caption format** | `City, Country - Month Day, Year: Factual sentence.`, with the real place from the image or supplied location. |
| **Active image** | Primary focus shown in Viewer; may be part of a larger selection. |
| **Anchor** | Starting image for Shift+Click range selection. |
| **Dirty** | Generated/saved metadata was edited and needs saving. |

---

## 20. Verification record

Last verified on the development machine (Windows 11, 2026-11-09):

- `cargo +stable-x86_64-pc-windows-gnu test` → **215 passed, 0 failed** (includes real ExifTool round-trips and the Windows inspector snapshot test).
- `cargo fmt --all -- --check`, `cargo check --all-targets` and `git diff --check` → success; no release executable was rebuilt in this pass (the existing release binary is unchanged).
- The latest release build was not launched in this pass; the UI verification refers to the existing GPUI build.
- UI verification is limited to the seven GPUI tests and eight inspector snapshots (six header crops and two full metadata rails); the full window, grid, Viewer, modals and file pickers were not visually driven in this pass.
- No live generation calls were made; provider tests use mock HTTP servers. Metadata correctness is covered by real ExifTool round-trips where ExifTool is installed.
- The GitHub Actions release workflow was added on 2026-11-09; it has not been run on GitHub yet.
