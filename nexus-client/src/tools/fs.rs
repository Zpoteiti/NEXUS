/// Filesystem tools: read_file, write_file, list_dir, stat.
/// All path operations are restricted by workspace policy (via `env::sanitize_path`).

pub use super::read_file::ReadFileTool;
pub use super::write_file::WriteFileTool;
pub use super::list_dir::ListDirTool;
pub use super::stat::StatTool;

#[cfg(test)]
mod tests {
    use crate::env;

    #[test]
    fn test_resolve_path_with_restrict_false_always_succeeds() {
        let result = env::sanitize_path("/tmp/test_file_xyz123", false);
        assert!(result.is_ok());
    }

    #[test]
    fn test_resolve_path_empty_string_accepted() {
        let result = env::sanitize_path("", false);
        assert!(result.is_ok());
    }
}
