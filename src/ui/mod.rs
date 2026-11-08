pub mod app_view;
pub mod tree;

pub use app_view::AutoWordingView;

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
