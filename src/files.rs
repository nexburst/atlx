//! Local file names for downloads.

/// The local name for a download whose name the server chose: the last path
/// component. Whoever uploaded the file picked it, so hidden names (a
/// `.bashrc` or `.envrc` planted in the current directory), drive or stream
/// separators and control characters are refused.
pub(crate) fn safe_file_name(title: &str) -> Option<&str> {
    let name = title.rsplit(['/', '\\']).next()?;
    let unsafe_name = name.trim().is_empty()
        || name.starts_with('.')
        || name.contains(':')
        || name.chars().any(char::is_control);
    (!unsafe_name).then_some(name)
}
