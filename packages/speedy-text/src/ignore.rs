use ignore::WalkBuilder;
use std::path::Path;

pub fn build_walker(root: &Path) -> WalkBuilder {
    let mut builder = WalkBuilder::new(root);
    builder
        .add_custom_ignore_filename(".speedyignore")
        .git_ignore(true)
        .git_global(false)
        .git_exclude(false)
        .hidden(false); // include hidden files (gitignore handles what to skip)
    builder
}
