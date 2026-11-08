/// Read the Windows app-theme preference used by Settings → Personalization.
#[cfg(windows)]
pub fn system_prefers_dark() -> bool {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = Command::new("reg");
    command.args([
        "query",
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
        "/v",
        "AppsUseLightTheme",
    ]);
    command.creation_flags(CREATE_NO_WINDOW);
    match command.output() {
        Ok(output) if output.status.success() => {
            prefers_dark_from_reg_output(&String::from_utf8_lossy(&output.stdout))
        }
        _ => false,
    }
}

#[cfg(not(windows))]
pub fn system_prefers_dark() -> bool {
    false
}

#[cfg(any(windows, test))]
fn prefers_dark_from_reg_output(output: &str) -> bool {
    output
        .lines()
        .find(|line| line.contains("AppsUseLightTheme"))
        .and_then(|line| line.split_whitespace().last())
        .map(|value| value.eq_ignore_ascii_case("0x0") || value == "0")
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_app_theme_registry_value_is_parsed() {
        assert!(prefers_dark_from_reg_output(
            "    AppsUseLightTheme    REG_DWORD    0x0"
        ));
        assert!(!prefers_dark_from_reg_output(
            "    AppsUseLightTheme    REG_DWORD    0x1"
        ));
        assert!(!prefers_dark_from_reg_output(""));
    }
}
