use crate::app::App;
use crate::ui::{ghost_button, tiny_button};
use directories::UserDirs;
use eframe::egui::{self, ScrollArea};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct FolderRow {
    pub path: PathBuf,
    pub name: String,
    pub depth: usize,
    pub expanded: bool,
    pub has_children: bool,
}

#[derive(Debug, Clone)]
struct FolderShortcut {
    name: &'static str,
    path: PathBuf,
}

/// Windows-style system folder tree. Mounted drives are the roots; expansion,
/// directory reads, and quick-access shortcuts are all lazy.
#[derive(Debug, Default)]
pub struct FolderTree {
    roots: Vec<PathBuf>,
    expanded: HashSet<String>,
    cache: HashMap<String, Vec<PathBuf>>,
}

impl FolderTree {
    pub fn system() -> Self {
        let mut tree = Self::default();
        tree.roots = system_roots();
        tree.sort_roots();
        tree
    }

    /// Rescan mounted drives while retaining any still-valid custom roots,
    /// such as an opened UNC share.
    pub fn rescan_roots(&mut self) {
        let mut roots = system_roots();
        for existing in &self.roots {
            if existing.is_dir()
                && !roots
                    .iter()
                    .any(|root| path_key(root) == path_key(existing))
            {
                roots.push(existing.clone());
            }
        }
        self.roots = roots;
        self.sort_roots();
        self.cache.clear();
    }

    /// Expand every folder from the nearest existing root down to `folder`.
    /// If the folder belongs to no known root, add its volume root.
    pub fn reveal(&mut self, folder: &Path) {
        let existing_root = self
            .roots
            .iter()
            .find(|root| path_is_within(folder, root))
            .cloned();

        let anchor = if let Some(root) = existing_root {
            root
        } else {
            let Some(volume) = volume_root(folder) else {
                return;
            };
            let volume = volume.to_path_buf();
            if !self
                .roots
                .iter()
                .any(|root| path_key(root) == path_key(&volume))
            {
                self.roots.push(volume.clone());
                self.sort_roots();
            }
            volume
        };

        for ancestor in folder.ancestors() {
            self.expanded.insert(path_key(ancestor));
            if path_key(ancestor) == path_key(&anchor) {
                break;
            }
        }
    }

    pub fn toggle(&mut self, path: &Path) {
        let key = path_key(path);
        if !self.expanded.remove(&key) {
            self.expanded.insert(key);
        }
    }

    pub fn refresh(&mut self) {
        self.cache.clear();
    }

    pub fn rows(&mut self) -> Vec<FolderRow> {
        let roots = self.roots.clone();
        let mut rows = Vec::new();
        for root in roots {
            self.push_rows(&root, 0, &mut rows);
        }
        rows
    }

    fn push_rows(&mut self, dir: &Path, depth: usize, rows: &mut Vec<FolderRow>) {
        let children = self.children(dir);
        let key = path_key(dir);
        let expanded = self.expanded.contains(&key);
        rows.push(FolderRow {
            path: dir.to_path_buf(),
            name: display_name(dir),
            depth,
            expanded,
            has_children: !children.is_empty(),
        });

        if expanded {
            for child in children {
                self.push_rows(&child, depth + 1, rows);
            }
        }
    }

    fn children(&mut self, dir: &Path) -> Vec<PathBuf> {
        let key = path_key(dir);
        if let Some(children) = self.cache.get(&key) {
            return children.clone();
        }

        let mut children: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .map(|name| {
                        let name = name.to_string_lossy();
                        !name.eq_ignore_ascii_case("$RECYCLE.BIN")
                            && !name.eq_ignore_ascii_case("System Volume Information")
                    })
                    .unwrap_or(false)
            })
            .collect();

        children.sort_by_key(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default()
        });
        self.cache.insert(key, children.clone());
        children
    }

    fn sort_roots(&mut self) {
        self.roots.sort_by_key(|path| path_key(path));
        self.roots
            .dedup_by(|left, right| path_key(left) == path_key(right));
    }

    #[cfg(test)]
    fn set_roots(&mut self, roots: Vec<PathBuf>) {
        self.roots = roots;
        self.sort_roots();
        self.cache.clear();
        self.expanded.clear();
        for root in &self.roots {
            self.expanded.insert(path_key(root));
        }
    }
}

fn path_key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

fn volume_root(path: &Path) -> Option<&Path> {
    path.ancestors().last()
}

fn path_is_within(path: &Path, root: &Path) -> bool {
    let path = path_key(path);
    let root = path_key(root);
    let root = root.trim_end_matches(['\\', '/']);
    path == root || path.starts_with(&format!("{root}\\")) || path.starts_with(&format!("{root}/"))
}

fn system_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for letter in b'A'..=b'Z' {
        let root = PathBuf::from(format!("{}:\\", letter as char));
        if root.is_dir() {
            roots.push(root);
        }
    }

    if roots.is_empty() {
        if let Ok(cwd) = std::env::current_dir() {
            if let Some(root) = volume_root(&cwd) {
                roots.push(root.to_path_buf());
            }
        }
    }
    roots
}

fn quick_access() -> Vec<FolderShortcut> {
    let Some(user_dirs) = UserDirs::new() else {
        return Vec::new();
    };

    let candidates = [
        ("Desktop", user_dirs.desktop_dir()),
        ("Downloads", user_dirs.download_dir()),
        ("Documents", user_dirs.document_dir()),
        ("Pictures", user_dirs.picture_dir()),
    ];

    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter_map(|(name, path)| {
            let path = path?;
            if !path.is_dir() || !seen.insert(path_key(path)) {
                return None;
            }
            Some(FolderShortcut {
                name,
                path: path.to_path_buf(),
            })
        })
        .collect()
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| path.display().to_string())
}

pub fn show(app: &mut App, ctx: &egui::Context, ui: &mut egui::Ui) {
    crate::ui::section_title(ui, "Folder Browser");
    ui.add_space(5.0);

    let parent = app.parent_folder();
    ui.horizontal(|ui| {
        if ui
            .add_enabled(parent.is_some(), ghost_button("Up"))
            .on_hover_text("Open the parent folder (Alt+Up)")
            .clicked()
        {
            app.open_parent_folder(ctx);
        }
        if ui.add(ghost_button("Open…")).clicked() {
            app.open_folder_dialog(ctx);
        }
        if ui.add(ghost_button("Refresh")).clicked() {
            app.refresh_browser(ctx);
        }
    });

    if let Some(path) = &app.current_folder {
        let full = path.display().to_string();
        ui.add_space(5.0);
        egui::Frame::none()
            .fill(crate::theme::chip_bg())
            .stroke(egui::Stroke::new(1.0_f32, crate::theme::border()))
            .rounding(egui::Rounding::same(7.0))
            .inner_margin(egui::Margin::symmetric(8.0, 4.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(&full)
                            .size(9.5)
                            .color(crate::theme::text_muted()),
                    )
                    .truncate(),
                )
                .on_hover_text(full);
            });
    }

    ui.add_space(8.0);

    let shortcuts = quick_access();
    let rows = app.folders.rows();
    let available = (ui.available_height() - 42.0).max(150.0);
    ScrollArea::vertical()
        .auto_shrink([false; 2])
        .max_height(available)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());

            if !shortcuts.is_empty() {
                group_label(ui, "QUICK ACCESS");
                ui.add_space(2.0);
                for shortcut in shortcuts {
                    let current = app.current_folder.as_deref() == Some(shortcut.path.as_path());
                    if ui
                        .selectable_label(
                            current,
                            egui::RichText::new(shortcut.name).size(11.5),
                        )
                        .on_hover_text(shortcut.path.display().to_string())
                        .clicked()
                    {
                        app.open_folder(ctx, shortcut.path);
                    }
                }
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);
            }

            group_label(ui, "THIS PC");
            ui.add_space(2.0);

            if rows.is_empty() {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new("No mounted drives were found.")
                            .size(11.0)
                            .color(crate::theme::text_muted()),
                    )
                    .wrap(),
                );
                return;
            }

            for row in rows {
                ui.horizontal(|ui| {
                    ui.add_space((row.depth as f32) * 13.0);
                    if row.has_children {
                        let glyph = if row.expanded { "▾" } else { "▸" };
                        if ui
                            .add(tiny_button(glyph))
                            .on_hover_text(if row.expanded {
                                "Collapse folder"
                            } else {
                                "Expand folder"
                            })
                            .clicked()
                        {
                            app.folders.toggle(&row.path);
                        }
                    } else {
                        ui.add_space(13.0);
                    }

                    let current = app.current_folder.as_deref() == Some(row.path.as_path());
                    if ui
                        .selectable_label(current, egui::RichText::new(&row.name).size(12.0))
                        .clicked()
                    {
                        app.open_folder(ctx, row.path.clone());
                    }

                    if current {
                        crate::ui::count_chip(
                            ui,
                            &format!("{}", app.folder_images.len()),
                            true,
                        );
                    }
                });
            }
        });

    ui.add_space(8.0);
    let width = ui.available_width();
    if ui
        .add_sized([width, 28.0], ghost_button("Add images…"))
        .clicked()
    {
        app.add_images_dialog(ctx);
    }
}

fn group_label(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(
        egui::RichText::new(text)
            .size(9.0)
            .strong()
            .color(crate::theme::text_muted()),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("autowording-tree-tests")
            .join(format!("{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn tree_lists_sorted_children_and_honours_expansion() {
        let root = temp_dir("rows");
        std::fs::create_dir_all(root.join("z_folder")).unwrap();
        std::fs::create_dir_all(root.join("a_folder")).unwrap();
        std::fs::create_dir_all(root.join("a_folder").join("nested")).unwrap();
        std::fs::create_dir_all(root.join(".hidden")).unwrap();

        let mut tree = FolderTree::default();
        tree.set_roots(vec![root.clone()]);
        let rows = tree.rows();
        assert_eq!(rows.len(), 4, "root + three visible children");
        assert_eq!(rows[0].name, root.file_name().unwrap().to_string_lossy());
        assert_eq!(rows[1].name, ".hidden");
        assert_eq!(rows[2].name, "a_folder");
        assert_eq!(rows[3].name, "z_folder");
        assert!(rows[0].expanded);
        assert!(rows[2].has_children);
        assert!(!rows[3].has_children);

        let a_folder = root.join("a_folder");
        tree.toggle(&a_folder);
        let rows = tree.rows();
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[3].name, "nested");
        assert_eq!(rows[3].depth, 2);

        tree.toggle(&a_folder);
        assert_eq!(tree.rows().len(), 4);
    }

    #[test]
    fn refresh_keeps_expansion_but_rereads_children() {
        let root = temp_dir("refresh");
        let child = root.join("first");
        std::fs::create_dir_all(&child).unwrap();
        let mut tree = FolderTree::default();
        tree.set_roots(vec![root]);
        tree.toggle(&child);
        assert_eq!(tree.rows().len(), 2);

        std::fs::create_dir_all(child.join("second")).unwrap();
        tree.refresh();
        let rows = tree.rows();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[2].name, "second");
    }

    #[test]
    fn reveal_expands_every_ancestor_to_the_folder() {
        let root = temp_dir("reveal");
        let child = root.join("child");
        let grandchild = child.join("grandchild");
        std::fs::create_dir_all(&grandchild).unwrap();

        let mut tree = FolderTree::default();
        tree.set_roots(vec![root.clone()]);
        tree.reveal(&grandchild);

        let rows = tree.rows();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].path, root);
        assert_eq!(rows[1].path, child);
        assert_eq!(rows[2].path, grandchild);
        assert!(rows.iter().all(|row| row.expanded));
        assert_eq!(tree.roots.len(), 1, "no extra volume root was added");
    }
}
