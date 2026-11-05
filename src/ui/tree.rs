use crate::app::App;
use crate::ui::{ghost_button, tiny_button};
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

/// Lazy folder tree: children are read only when a folder is expanded.
#[derive(Debug, Default)]
pub struct FolderTree {
    root: Option<PathBuf>,
    expanded: HashSet<PathBuf>,
    cache: HashMap<PathBuf, Vec<PathBuf>>,
}

impl FolderTree {
    pub fn set_root(&mut self, root: &Path) {
        self.root = Some(root.to_path_buf());
        self.expanded.clear();
        self.cache.clear();
        self.expanded.insert(root.to_path_buf());
    }

    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    pub fn toggle(&mut self, path: &Path) {
        let path = path.to_path_buf();
        if !self.expanded.remove(&path) {
            self.expanded.insert(path);
        }
    }

    pub fn refresh(&mut self) {
        self.cache.clear();
    }

    pub fn rows(&mut self) -> Vec<FolderRow> {
        let Some(root) = self.root.clone() else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        self.push_rows(&root, 0, &mut rows);
        rows
    }

    fn push_rows(&mut self, dir: &Path, depth: usize, rows: &mut Vec<FolderRow>) {
        let children = self.children(dir);
        let has_children = !children.is_empty();
        rows.push(FolderRow {
            path: dir.to_path_buf(),
            name: display_name(dir),
            depth,
            expanded: self.expanded.contains(dir),
            has_children,
        });

        if self.expanded.contains(dir) {
            for child in children {
                self.push_rows(&child, depth + 1, rows);
            }
        }
    }

    fn children(&mut self, dir: &Path) -> Vec<PathBuf> {
        if let Some(children) = self.cache.get(dir) {
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
                        !name.starts_with('.')
                            && !matches!(
                                name.as_ref(),
                                "target"
                                    | "node_modules"
                                    | "$RECYCLE.BIN"
                                    | "System Volume Information"
                            )
                    })
                    .unwrap_or(false)
            })
            .collect();

        children.sort_by_key(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default()
        });
        self.cache.insert(dir.to_path_buf(), children.clone());
        children
    }
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| path.display().to_string())
}

pub fn show(app: &mut App, ctx: &egui::Context, ui: &mut egui::Ui) {
    crate::ui::section_title(ui, "Folder Browser");
    ui.add_space(4.0);

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
            app.refresh_folder(ctx);
        }
    });

    if let Some(path) = &app.current_folder {
        let full = path.display().to_string();
        ui.add_space(3.0);
        ui.add(
            egui::Label::new(
                egui::RichText::new(&full)
                    .size(9.5)
                    .color(crate::theme::TEXT_MUTED),
            )
            .truncate(),
        )
        .on_hover_text(full);
    }

    ui.add_space(6.0);

    if app.folders.root().is_none() {
        ui.add(
            egui::Label::new(
                egui::RichText::new(
                    "No folder open. Choose a stock folder to populate the grid.",
                )
                .size(11.0)
                .color(crate::theme::TEXT_MUTED),
            )
            .wrap(),
        );
    } else {
        let rows = app.folders.rows();
        let available = (ui.available_height() - 76.0).max(120.0);
        ScrollArea::vertical()
            .auto_shrink([false; 2])
            .max_height(available)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
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
                            .selectable_label(
                                current,
                                egui::RichText::new(&row.name).size(12.0),
                            )
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
    }

    ui.add_space(6.0);
    if ui.add(ghost_button("Add images…")).clicked() {
        app.add_images_dialog(ctx);
    }
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
        tree.set_root(&root);
        let rows = tree.rows();
        assert_eq!(rows.len(), 3, "root + two visible children");
        assert_eq!(rows[0].name, root.file_name().unwrap().to_string_lossy());
        assert_eq!(rows[1].name, "a_folder");
        assert_eq!(rows[2].name, "z_folder");
        assert!(rows[0].expanded);
        assert!(rows[1].has_children);
        assert!(!rows[2].has_children);

        let a_folder = root.join("a_folder");
        tree.toggle(&a_folder);
        let rows = tree.rows();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[2].name, "nested");
        assert_eq!(rows[2].depth, 2);

        tree.toggle(&a_folder);
        assert_eq!(tree.rows().len(), 3);
    }

    #[test]
    fn refresh_keeps_expansion_but_rereads_children() {
        let root = temp_dir("refresh");
        let child = root.join("first");
        std::fs::create_dir_all(&child).unwrap();
        let mut tree = FolderTree::default();
        tree.set_root(&root);
        tree.toggle(&child);
        assert_eq!(tree.rows().len(), 2);

        std::fs::create_dir_all(child.join("second")).unwrap();
        tree.refresh();
        let rows = tree.rows();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[2].name, "second");
    }
}
