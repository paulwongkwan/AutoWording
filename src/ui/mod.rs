pub mod app_view;
pub mod tree;

pub use app_view::AutoWordingView;

gpui_kit::actions!(
    autowording,
    [ToggleInspector, PauseBatch, ResumeBatch, CancelBatch]
);

pub fn init_keybindings(cx: &mut gpui_kit::App) {
    cx.bind_keys([
        gpui_kit::KeyBinding::new("ctrl-b", ToggleInspector, Some("AutoWording")),
        gpui_kit::KeyBinding::new("ctrl-p", PauseBatch, Some("AutoWording")),
        gpui_kit::KeyBinding::new("ctrl-shift-p", ResumeBatch, Some("AutoWording")),
        gpui_kit::KeyBinding::new("ctrl-shift-c", CancelBatch, Some("AutoWording")),
    ]);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiAction {
    OpenFolder,
    AddImages,
    SaveSelected,
    RestoreBackup,
    ExportCsv(crate::csv::Agency),
    ImportCsv,
    GenerateSelected,
    SelectAll,
    ClearSelection,
    Refresh,
    CheckExif,
    About,
    Quit,
}
